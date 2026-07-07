//! The read-only diff preview shown in the centre pane. This module owns the whole
//! preview subsystem: the [`DiffView`] data model (a parsed, render-ready diff or a
//! decoded image), the unified-diff parser ([`parse_diff`]) that classifies and
//! syntax-highlights each line, and the image path ([`PreviewImage`]) that decodes a
//! changed picture and rasterizes it to sixel / half-block cells.
//!
//! It's built and kept in sync by the git panel ([`git`](super::git)) — whose `impl App`
//! methods open, follow and refresh the preview — and drawn by
//! [`view::pane`](super::view::pane). It is **not** a [`Session`](super::Session): there's
//! no PTY, just text and pixels we render and scroll ourselves.

use crate::git::{self, Commit, FileEntry};
use ratatui::style::Color;
use std::path::Path;
use std::time::Instant;

use super::highlight::Highlighter;

/// A read-only diff of one changed file, shown in the centre pane (where an agent
/// usually lives) as a live preview of the file under the Changes cursor. It is not
/// a [`Session`](super::Session) — there's no PTY, just parsed `git diff` text we
/// draw ourselves and scroll on our own. Built on click / `v`, kept in sync as the
/// cursor moves, and dropped when a session is selected (see [`App::diff_upkeep`]).
pub(crate) struct DiffView {
    /// Which project's repo this diff belongs to — so a project switch invalidates it.
    pub project: usize,
    /// The changed-file path it shows (also its identity for the live refresh).
    pub path: String,
    /// Added / removed line counts, for the header (`+N −M`).
    pub added: u32,
    pub removed: u32,
    /// The classified, header-stripped diff body.
    pub lines: Vec<DiffLine>,
    /// Width (in digits) of the new-file line-number gutter — the largest number shown,
    /// so the renderer can right-align the column. 0 when there are no numbered lines.
    pub gutter: usize,
    /// First visible line (the pager scroll offset).
    pub scroll: usize,
    /// A decoded image, when the changed file is a picture: the pane shows it instead
    /// of a textual diff (and `lines` is empty). See [`PreviewImage`]. Only ever set for
    /// a working-file diff — a commit's historical blob isn't decoded.
    pub image: Option<PreviewImage>,
    /// When set, this pager is showing a single commit (`git show`) rather than the
    /// working-tree file diff: its short hash + subject for the title. A commit diff is
    /// static and multi-file, so it never self-refreshes and `path` is empty.
    pub commit: Option<CommitRef>,
    /// When the body was last built, to throttle the live re-read.
    pub(crate) built_at: Instant,
}

/// The identity of a commit shown in the pager — just what the title needs.
pub(crate) struct CommitRef {
    pub short: String,
    pub subject: String,
}

/// One diff body line: its change kind, the raw text (kept for the hunk header and
/// width), the new-file line number for the gutter, and the syntax-highlighted code
/// (the leading `+`/`-`/space sign stripped off — the renderer draws that itself).
pub(crate) struct DiffLine {
    pub text: String,
    pub kind: DiffKind,
    /// The leading diff sign — `+`, `-`, or a space for context (unused for a hunk).
    pub sign: char,
    /// New-file line number for the gutter; `None` for a deletion or a hunk header.
    pub new_no: Option<u32>,
    /// Highlighted code segments (foreground colour + text), sign already stripped.
    /// Empty for a hunk header (drawn from `text`).
    pub spans: Vec<(Color, String)>,
}

impl DiffLine {
    /// The selectable / copyable text of the line: the code with the leading `+`/`-`/
    /// space sign stripped (a hunk header keeps its full `@@ … @@` text). This is what a
    /// drag-select yields and highlights, deliberately excluding the gutter line number
    /// and the sign column so a copied diff pastes as plain code.
    pub(crate) fn content(&self) -> &str {
        match self.kind {
            // A hunk header and a file separator carry their full text, sign and all.
            DiffKind::Hunk | DiffKind::File => &self.text,
            _ => self.text.get(1..).unwrap_or(""),
        }
    }
}

/// The visible diff line kinds. The `index`/`+++`/`---` header noise is dropped at build
/// time; a `diff --git` header only becomes a [`File`](DiffKind::File) divider for a
/// multi-file commit (`git show`), and is dropped for the single-file working preview.
#[derive(Clone, Copy)]
pub(crate) enum DiffKind {
    Add,
    Del,
    Hunk,
    Context,
    /// A `diff --git` file boundary in a multi-file commit — rendered as a bold path
    /// divider so you can tell which file each hunk belongs to.
    File,
}

impl DiffView {
    /// Build the working-tree preview of one changed `file` (`git diff HEAD`). An image
    /// gets its picture shown instead of git's "Binary files differ"; anything else is
    /// parsed into render-ready lines. Single-file, so the `diff --git` header is dropped
    /// for a clean read (no file dividers).
    pub(crate) fn build(project: usize, dir: &Path, file: &FileEntry) -> DiffView {
        // Falls through to the text path when it isn't a decodable format, is missing
        // (a deletion), is too big, or fails to decode.
        if is_image_path(&file.path) {
            if let Some(image) = PreviewImage::load(dir, &file.path) {
                return DiffView {
                    project,
                    path: file.path.clone(),
                    added: 0,
                    removed: 0,
                    lines: Vec::new(),
                    gutter: 0,
                    scroll: 0,
                    image: Some(image),
                    commit: None,
                    built_at: Instant::now(),
                };
            }
        }
        let raw = git::diff(dir, &file.path, file.untracked);
        let (lines, added, removed, max_no) = parse_diff(&raw, Some(&file.path), false);
        DiffView {
            project,
            path: file.path.clone(),
            added,
            removed,
            lines,
            gutter: digits(max_no),
            scroll: 0,
            image: None,
            commit: None,
            built_at: Instant::now(),
        }
    }

    /// Build the pager for a single commit (`git show`). Static and multi-file, so it
    /// renders the `diff --git` file dividers, carries the commit's hash + subject for the
    /// title, and never previews a binary as an image (that would need the historical blob).
    pub(crate) fn build_commit(project: usize, dir: &Path, c: &Commit) -> DiffView {
        let raw = git::show(dir, &c.hash);
        let (lines, added, removed, max_no) = parse_diff(&raw, None, true);
        DiffView {
            project,
            path: String::new(),
            added,
            removed,
            lines,
            gutter: digits(max_no),
            scroll: 0,
            image: None,
            commit: Some(CommitRef { short: c.short.clone(), subject: c.summary.clone() }),
            built_at: Instant::now(),
        }
    }
}

/// Parse raw unified-diff text into render-ready [`DiffLine`]s, with the added/removed
/// tallies and the widest new-file line number (→ the gutter width). Once inside a hunk a
/// leading `+`/`-` is unambiguously an addition/deletion (the `+++`/`---` headers only
/// appear *before* the first `@@`), so a simple in-hunk flag classifies every line.
/// `initial` seeds the syntax highlighter with a file path; with `file_headers` set (a
/// multi-file commit) each `diff --git … b/<path>` becomes a bold [`DiffKind::File`]
/// divider and re-seeds the highlighter to that file's language.
fn parse_diff(
    raw: &str,
    initial: Option<&str>,
    file_headers: bool,
) -> (Vec<DiffLine>, u32, u32, u32) {
    // Don't light a giant diff — lighting every line costs more than it's worth for a
    // preview, so past the cap everything falls back to plain text.
    let plain = raw.len() > MAX_HIGHLIGHT_BYTES;
    let hl_for = |path: Option<&str>| match (plain, path) {
        (false, Some(p)) => Highlighter::for_path(p),
        _ => Highlighter::plain(),
    };
    let mut hl = hl_for(initial);
    let mut lines = Vec::new();
    let (mut added, mut removed) = (0u32, 0u32);
    let mut in_hunk = false;
    let mut new_no: u32 = 0; // next new-file line number within the current hunk
    let mut max_no: u32 = 0; // widest number shown → the gutter width
    for l in raw.lines() {
        if let Some(body) = l.strip_prefix("diff --git ") {
            in_hunk = false; // a new file section — back to header noise
            let path = git_b_path(body);
            hl = hl_for(path.as_deref()); // colour each file by its own language
            if file_headers {
                lines.push(DiffLine {
                    text: path.unwrap_or_else(|| l.to_string()),
                    kind: DiffKind::File,
                    sign: ' ',
                    new_no: None,
                    spans: Vec::new(),
                });
            }
        } else if l.starts_with("diff ") {
            in_hunk = false; // other diff header forms (`--cc`, `--combined`)
        } else if l.starts_with("@@") {
            in_hunk = true;
            // Pick up the new-file start line so the gutter can number the hunk.
            new_no = hunk_new_start(l).unwrap_or(new_no);
            lines.push(DiffLine {
                text: l.to_string(),
                kind: DiffKind::Hunk,
                sign: ' ',
                new_no: None,
                spans: Vec::new(),
            });
        } else if l.starts_with("Binary files") {
            lines.push(DiffLine {
                text: l.to_string(),
                kind: DiffKind::Context,
                sign: ' ',
                new_no: None,
                spans: hl.line(l),
            });
        } else if in_hunk {
            let sign = l.as_bytes().first().copied();
            let code = l.get(1..).unwrap_or(""); // the line minus its sign column
            let (kind, no) = match sign {
                Some(b'+') => {
                    added += 1;
                    let n = new_no;
                    new_no += 1;
                    (DiffKind::Add, Some(n))
                }
                Some(b'-') => {
                    removed += 1;
                    (DiffKind::Del, None) // a deletion has no new-file line
                }
                // git's "\ No newline at end of file" note: a marker, not a real
                // line — show it but don't number it or advance the counter.
                Some(b'\\') => (DiffKind::Context, None),
                _ => {
                    let n = new_no;
                    new_no += 1;
                    (DiffKind::Context, Some(n))
                }
            };
            if let Some(n) = no {
                max_no = max_no.max(n);
            }
            lines.push(DiffLine {
                text: l.to_string(),
                kind,
                sign: sign.map(|b| b as char).unwrap_or(' '),
                new_no: no,
                spans: hl.line(code),
            });
        }
        // else: header lines before the first hunk — hidden for a clean read.
    }
    (lines, added, removed, max_no)
}

/// The new-file path from a `diff --git a/… b/…` header body (everything after
/// `"diff --git "`). Splits on the last `" b/"` — unambiguous for the unquoted paths git
/// emits (a path with spaces gets quoted, which we don't try to parse, falling back to the
/// raw header). Returns the `b/`-side path with its prefix stripped.
fn git_b_path(body: &str) -> Option<String> {
    body.rsplit_once(" b/").map(|(_, b)| b.to_string())
}

/// Don't syntax-highlight a diff whose raw text exceeds this — lighting thousands of
/// lines (each parsed on its own, see [`highlight`](super::highlight)) isn't worth it
/// for a preview pane, so past the cap the pager falls back to plain text.
const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;

/// The new-file starting line of a hunk header — the `+c` in `@@ -a,b +c,d @@`.
fn hunk_new_start(header: &str) -> Option<u32> {
    let plus = header.split_whitespace().find(|t| t.starts_with('+'))?;
    plus.trim_start_matches('+').split(',').next()?.parse().ok()
}

/// The number of decimal digits in `n` (so `0` and `9` are one wide, `10` two).
fn digits(n: u32) -> usize {
    // `checked_ilog10` is `None` for 0; the `unwrap_or(0)` maps that to a single digit.
    n.checked_ilog10().unwrap_or(0) as usize + 1
}

/// The image extensions we inline-preview — kept in step with the decoders enabled on
/// the `image` crate in `Cargo.toml`. Matched case-insensitively.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Don't decode anything bigger than this on disk: the decode runs on the UI thread,
/// so a stray giant asset shouldn't stall it. Decompression is separately bounded by
/// the `image::Limits` in [`PreviewImage::load`] (against small-but-huge-when-decoded
/// files).
const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;

/// Whether `path`'s extension is one of the [`IMAGE_EXTS`] we can preview.
fn is_image_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.as_str()))
}

/// One rendered image cell, drawn as `▀` (upper-half block): its top half takes the
/// foreground colour and its bottom half the background, so a single character carries
/// two vertically-stacked pixels — doubling the effective vertical resolution.
#[derive(Clone, Copy)]
pub(crate) struct HalfCell {
    pub top: (u8, u8, u8),
    pub bottom: (u8, u8, u8),
}

/// A decoded image shown in the diff pane in place of a textual diff. It's rendered as
/// half-block coloured text straight into the ratatui buffer (see [`super::view`]), so
/// it needs no terminal graphics protocol
/// (Kitty/Sixel/iTerm2) and survives the tmux jail mmux draws through — it's just
/// styled cells like the rest of the UI. The source pixels are decoded once; the
/// half-block grid is re-rasterized only when the pane's cell size changes (the draw
/// loop repaints ~20×/s, so per-frame resizing would be wasteful).
pub(crate) struct PreviewImage {
    /// Decoded source pixels, resized on demand for the current pane size.
    src: image::RgbaImage,
    /// Natural pixel dimensions, shown in the pane title.
    pub dims: (u32, u32),
    /// Cached half-block rasterization for the last `(cols, rows)` it was drawn at
    /// (the fallback path, when the terminal can't do sixel).
    cache: Option<(u16, u16, Vec<Vec<HalfCell>>)>,
    /// Cached sixel encoding for the last `(cols, rows)` — the real-pixel path. `None`
    /// inside the tuple means the encode failed (cached so we don't retry every frame).
    sixel_cache: Option<(u16, u16, Option<String>)>,
}

impl PreviewImage {
    /// Decode the working-tree copy of `rel` under `dir`. Returns `None` (→ the text
    /// diff path) when the file is missing, over [`MAX_IMAGE_BYTES`], or not a decodable
    /// image. Both a byte cap and `image::Limits` bound the work, since it runs inline
    /// on the UI thread.
    fn load(dir: &Path, rel: &str) -> Option<PreviewImage> {
        let path = dir.join(rel);
        if std::fs::metadata(&path).ok()?.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let mut reader = image::ImageReader::open(&path).ok()?.with_guessed_format().ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(20_000);
        limits.max_image_height = Some(20_000);
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        let src = reader.decode().ok()?.to_rgba8();
        let dims = (src.width(), src.height());
        if dims.0 == 0 || dims.1 == 0 {
            return None;
        }
        Some(PreviewImage { src, dims, cache: None, sixel_cache: None })
    }

    /// The sixel encoding sized to fit `cols`×`rows` cells given the terminal's
    /// `cell_px` pixel-per-cell, cached by target size so it re-encodes only on a
    /// resize (encoding + colour quantization is far too costly to run per frame).
    /// `None` when encoding fails. See [`super::view`] and `run_loop` for how it's drawn.
    pub(crate) fn sixel(&mut self, cols: u16, rows: u16, cell_px: (u16, u16)) -> Option<&str> {
        if self.sixel_cache.as_ref().map(|(w, h, _)| (*w, *h)) != Some((cols, rows)) {
            let encoded = encode_sixel(&self.src, cols, rows, cell_px);
            self.sixel_cache = Some((cols, rows, encoded));
        }
        self.sixel_cache.as_ref().and_then(|(_, _, s)| s.as_deref())
    }

    /// The half-block grid sized to fit `cols`×`rows` cells (aspect preserved),
    /// re-rasterizing only when that size changed. Rows are equal width and the grid
    /// may be smaller than the target area, so the caller centres it.
    pub(crate) fn grid(&mut self, cols: u16, rows: u16) -> &[Vec<HalfCell>] {
        if self.cache.as_ref().map(|(w, h, _)| (*w, *h)) != Some((cols, rows)) {
            let cells = rasterize(&self.src, cols, rows);
            self.cache = Some((cols, rows, cells));
        }
        &self.cache.as_ref().unwrap().2
    }
}

/// Resize `src` to fit the `cols`×`rows` cell area at `cell_px` pixels-per-cell
/// (aspect preserved; sharp Lanczos downscale so text stays as legible as the pixel
/// budget allows) and encode it as a sixel string. `None` if encoding fails.
fn encode_sixel(src: &image::RgbaImage, cols: u16, rows: u16, cell_px: (u16, u16)) -> Option<String> {
    let (cw, ch) = (cell_px.0.max(1) as u32, cell_px.1.max(1) as u32);
    let (avail_w, avail_h) = (cols as u32 * cw, rows as u32 * ch);
    if avail_w == 0 || avail_h == 0 {
        return None;
    }
    let (iw, ih) = (src.width() as f64, src.height() as f64);
    let scale = (avail_w as f64 / iw).min(avail_h as f64 / ih);
    let nw = ((iw * scale).round() as u32).max(1);
    let nh = ((ih * scale).round() as u32).max(1);
    // Lanczos when shrinking (keeps edges/text crisp), nearest when blowing up a small
    // icon (no muddy interpolation).
    let filter = if scale < 1.0 {
        image::imageops::FilterType::Lanczos3
    } else {
        image::imageops::FilterType::Nearest
    };
    let resized = image::imageops::resize(src, nw, nh, filter);
    icy_sixel::SixelImage::from_rgba(resized.into_raw(), nw as usize, nh as usize)
        .encode()
        .ok()
}

/// Resize `src` to fit `cols`×`2·rows` pixels (aspect preserved — a terminal cell is
/// ~1:2, so two stacked pixels read as square — smoothly downscaled, crisply upscaled)
/// and fold each vertical pixel pair into one [`HalfCell`], compositing any alpha over
/// black.
fn rasterize(src: &image::RgbaImage, cols: u16, rows: u16) -> Vec<Vec<HalfCell>> {
    let (max_w, max_h) = (cols as u32, rows as u32 * 2);
    if max_w == 0 || max_h == 0 {
        return Vec::new();
    }
    let (iw, ih) = (src.width() as f64, src.height() as f64);
    let scale = (max_w as f64 / iw).min(max_h as f64 / ih);
    let nw = ((iw * scale).round() as u32).max(1);
    let nh = ((ih * scale).round() as u32).max(1);
    let filter = if scale < 1.0 {
        image::imageops::FilterType::Triangle
    } else {
        image::imageops::FilterType::Nearest
    };
    let img = image::imageops::resize(src, nw, nh, filter);
    // Flatten alpha over black so half-transparent pixels don't render as opaque.
    let over_black = |p: &image::Rgba<u8>| {
        let a = p.0[3] as u32;
        let c = |i: usize| ((p.0[i] as u32 * a) / 255) as u8;
        (c(0), c(1), c(2))
    };
    (0..nh.div_ceil(2))
        .map(|cy| {
            (0..nw)
                .map(|x| {
                    let top = over_black(img.get_pixel(x, cy * 2));
                    let by = cy * 2 + 1;
                    // An odd height leaves the last cell's bottom pixel empty → black.
                    let bottom =
                        if by < nh { over_black(img.get_pixel(x, by)) } else { (0, 0, 0) };
                    HalfCell { top, bottom }
                })
                .collect()
        })
        .collect()
}
