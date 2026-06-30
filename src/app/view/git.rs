//! Rendering for the native git panel.
//!
//! The right column is three stacked, individually-bordered boxes — **Changes**
//! (top, flexible, rendered as a compressed directory *tree* via
//! [`git::tree_rows`](crate::git::tree_rows)), **Branches** (middle), **Recent**
//! (bottom) — plus the modal
//! commit / new-branch prompt drawn over the whole UI. These are free functions
//! over `&GitPanel` so they stay decoupled from the rest of `App`; the thin
//! wrapper that owns the per-frame hit [`Regions`](super::Regions) lives in
//! [`super::pane`] and stores the row maps [`render_git`] returns.

use crate::app::git::{GitPanel, Overlay, PromptKind, Section};
use crate::app::UpdateState;
use crate::app::linkbrowse::{DirEntry, LinkBrowser, Preview};
use crate::app::picker::Picker;
use crate::app::procform::{ProcForm, Step, STEPS};
use crate::git::{Branch, Commit, FileEntry, Stage, TreeRow};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// The clickable rows the panel exposes this frame, for mouse routing.
#[derive(Default)]
pub(crate) struct GitRows {
    pub rows: Vec<(u16, usize)>,     // tree rows: screen y → cursor (tree-row) index
    pub branches: Vec<(u16, usize)>, // branch rows: screen y → branch index
    pub changes_area: Option<Rect>,  // the Changes box, so a whitespace click focuses it
    pub branches_area: Option<Rect>, // the Branches box, ditto
}

/// Draw the git panel into `area` as three bordered boxes, returning the file and
/// branch row maps for click routing. `compact_bar` prefixes the top box with ☰.
pub(crate) fn render_git(
    f: &mut Frame,
    area: Rect,
    git: &GitPanel,
    focused: bool,
    compact_bar: bool,
) -> GitRows {
    let mut hit = GitRows::default();
    if area.width < 3 || area.height < 3 {
        return hit;
    }

    // Branches + Recent are equal-height side boxes; Changes takes the rest. On a
    // short column we drop the side boxes and give everything to Changes.
    let side_h: u16 = if area.height >= 20 {
        8
    } else if area.height >= 13 {
        5
    } else {
        0
    };
    let constraints = if side_h == 0 {
        vec![Constraint::Min(1)]
    } else {
        vec![
            Constraint::Min(3),
            Constraint::Length(side_h),
            Constraint::Length(side_h),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    hit.changes_area = Some(chunks[0]);
    render_changes(f, chunks[0], git, focused, compact_bar, &mut hit.rows);
    if side_h > 0 {
        hit.branches_area = Some(chunks[1]);
        render_branches(f, chunks[1], git, focused, &mut hit.branches);
        render_recent(f, chunks[2], git);
    }
    hit
}

/// A bordered box; `active` paints the border magenta (the focused section),
/// otherwise dim.
fn boxed(title: String, active: bool) -> Block<'static> {
    let border = if active { Color::Magenta } else { Color::DarkGray };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border))
}

fn render_changes(
    f: &mut Frame,
    area: Rect,
    git: &GitPanel,
    focused: bool,
    compact_bar: bool,
    hit: &mut Vec<(u16, usize)>,
) {
    let active = focused && git.section == Section::Changes;
    let mut title = String::new();
    if compact_bar {
        title.push_str(" ☰");
    }
    title.push_str(" Changes ");
    if !git.branch.is_empty() {
        title.push_str(&format!("· {}", git.branch));
        if git.ahead > 0 {
            title.push_str(&format!(" ↑{}", git.ahead));
        }
        if git.behind > 0 {
            title.push_str(&format!(" ↓{}", git.behind));
        }
        title.push(' ');
    }
    if let Some(b) = git.busy {
        title.push_str(&format!("· {b} "));
    }
    let block = boxed(title, active);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if git.rows.is_empty() {
        f.render_widget(dim_line("  working tree clean"), inner);
        return;
    }
    // Window over the precomputed tree so the cursor's row stays on screen. Every row
    // (root / dir / file) is selectable, so every visible one is registered for clicks.
    let root_label = git
        .dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("/");
    let (start, count) = window(git.rows.len(), git.cursor, inner.height as usize);
    let mut lines = Vec::with_capacity(count);
    for off in 0..count {
        let i = start + off;
        let selected = active && i == git.cursor;
        let line = match &git.rows[i] {
            TreeRow::Root { staged } => node_row(root_label, 0, *staged, selected, inner.width),
            TreeRow::Dir { label, depth, staged, .. } => {
                node_row(label, *depth, *staged, selected, inner.width)
            }
            TreeRow::File { idx, depth } => {
                file_row(&git.files[*idx], *depth, selected, inner.width)
            }
        };
        lines.push(line);
        hit.push((inner.y + off as u16, i));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_branches(
    f: &mut Frame,
    area: Rect,
    git: &GitPanel,
    focused: bool,
    hit: &mut Vec<(u16, usize)>,
) {
    let active = focused && git.section == Section::Branches;
    let block = boxed(" Branches ".into(), active);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if git.branches.is_empty() {
        f.render_widget(dim_line("  no branches"), inner);
        return;
    }
    let (start, count) = window(git.branches.len(), git.branch_cursor, inner.height as usize);
    let mut lines = Vec::with_capacity(count);
    for off in 0..count {
        let i = start + off;
        lines.push(branch_row(&git.branches[i], active && i == git.branch_cursor, inner.width));
        hit.push((inner.y + off as u16, i));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_recent(f: &mut Frame, area: Rect, git: &GitPanel) {
    let block = boxed(" Recent ".into(), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines: Vec<Line> = git
        .log
        .iter()
        .take(inner.height as usize)
        .map(commit_row)
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// Pad a selected row with trailing spaces and paint the highlight background so the
/// selection bar spans the full column width (matching the sidebar's `entry_line`),
/// rather than stopping at the end of the name.
fn fill_selection(line: &mut Line<'static>, width: u16) {
    let pad = width.saturating_sub(line.width() as u16);
    if pad > 0 {
        line.spans.push(Span::raw(" ".repeat(pad as usize)));
    }
    line.style = Style::default().bg(Color::Rgb(45, 45, 60));
}

/// A directory (or the whole-repo root) row: selection bar, indentation, an aggregate
/// staging checkbox, then the (possibly chain-compressed) name with a trailing slash.
fn node_row(label: &str, depth: usize, staged: Stage, selected: bool, width: u16) -> Line<'static> {
    let bar = if selected { "▌" } else { " " };
    let indent = "  ".repeat(depth);
    let (mark, mark_style) = checkbox(staged);
    let chrome = 6 + indent.chars().count(); // bar + "[x]" + space + "/"
    let name = truncate_middle(label, (width as usize).saturating_sub(chrome));
    let mut line = Line::from(vec![
        Span::styled(bar.to_string(), Style::default().fg(Color::Magenta)),
        Span::raw(indent),
        Span::styled(mark, mark_style),
        Span::styled(format!(" {name}/"), Style::default().fg(Color::Blue)),
    ]);
    if selected {
        fill_selection(&mut line, width);
    }
    line
}

fn file_row(file: &FileEntry, depth: usize, selected: bool, width: u16) -> Line<'static> {
    let bar = if selected { "▌" } else { " " };
    let indent = "  ".repeat(depth);
    // Checkbox shows staging; the filename's colour shows the change type.
    let stage = if file.staged && file.unstaged {
        Stage::Partial
    } else if file.staged {
        Stage::All
    } else {
        Stage::None
    };
    let (mark, mark_style) = checkbox(stage);
    let leaf = file.path.rsplit('/').next().unwrap_or(&file.path);
    let chrome = 5 + indent.chars().count(); // bar + "[x]" + space
    let name = truncate_middle(leaf, (width as usize).saturating_sub(chrome));
    let mut line = Line::from(vec![
        Span::styled(bar.to_string(), Style::default().fg(Color::Magenta)),
        Span::raw(indent),
        Span::styled(mark, mark_style),
        Span::styled(format!(" {name}"), Style::default().fg(change_color(file))),
    ]);
    if selected {
        fill_selection(&mut line, width);
    }
    line
}

/// The staging checkbox: `[✓]` fully staged, `[~]` partially, `[ ]` unstaged.
fn checkbox(staged: Stage) -> (&'static str, Style) {
    match staged {
        Stage::All => ("[✓]", Style::default().fg(Color::Green)),
        Stage::Partial => ("[~]", Style::default().fg(Color::Yellow)),
        Stage::None => ("[ ]", Style::default().fg(Color::DarkGray)),
    }
}

/// The filename colour encoding the change type (staging is shown by the checkbox).
fn change_color(file: &FileEntry) -> Color {
    if file.untracked {
        return Color::Red;
    }
    match file.glyph {
        'A' => Color::Green,            // added
        'D' | 'U' => Color::Red,        // deleted / unmerged
        'R' | 'C' => Color::Cyan,       // renamed / copied
        _ => Color::Yellow,             // modified & friends
    }
}

fn branch_row(b: &Branch, selected: bool, width: u16) -> Line<'static> {
    let avail = (width as usize).saturating_sub(3); // bar + dot + space
    let bar = if selected { "▌" } else { " " };
    let dot = if b.current { "●" } else { " " };
    let name_style = if b.current {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut spans = vec![
        Span::styled(bar.to_string(), Style::default().fg(Color::Magenta)),
        Span::styled(format!("{dot} "), Style::default().fg(Color::Green)),
        Span::styled(truncate_left(&b.name, avail), name_style),
    ];
    if !b.track.is_empty() {
        spans.push(Span::styled(
            format!("  {}", b.track),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let mut line = Line::from(spans);
    if selected {
        fill_selection(&mut line, width);
    }
    line
}

fn commit_row(c: &Commit) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{} ", c.short), Style::default().fg(Color::DarkGray)),
        Span::styled(c.summary.clone(), Style::default().fg(Color::Gray)),
    ])
}

fn dim_line(s: &str) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        s.to_string(),
        Style::default().fg(Color::DarkGray),
    )))
}

/// The window of `len` items to show in `rows` lines, keeping `cursor` visible.
fn window(len: usize, cursor: usize, rows: usize) -> (usize, usize) {
    if rows == 0 {
        return (0, 0);
    }
    if len <= rows {
        return (0, len);
    }
    let start = cursor.saturating_sub(rows / 2).min(len - rows);
    (start, rows)
}

// ---- overlay: the commit / new-branch text prompt ----

pub(crate) fn render_overlay(f: &mut Frame, area: Rect, ov: &Overlay) {
    match ov {
        Overlay::Prompt { title, buf, kind } => render_prompt(f, area, title, buf, *kind),
        Overlay::Confirm { title, body, hint, .. } => render_confirm(f, area, title, body, hint),
        Overlay::Picker(p) => render_picker(f, area, p),
        Overlay::NewProcess(form) => render_procform(f, area, form),
        Overlay::LinkProject(b) => render_linkbrowse(f, area, b),
        // Drawn by `render_about` (it needs live update state), routed there in `draw`.
        Overlay::About => {}
    }
}

/// The "About mmux" card: the version, the project's home + source links (a quiet
/// backlink to the author), and a live self-update status line with the keys to check
/// or apply. `can_update` is the synchronous permitted-gate; the Homebrew-managed test
/// is the worker's job, so a permitted build reads as up to date until a check says
/// otherwise. Routed here from [`super::App::draw`] so it can see `self.update`.
pub(crate) fn render_about(f: &mut Frame, area: Rect, update: &UpdateState, can_update: bool) {
    let version = env!("CARGO_PKG_VERSION");

    // The update status line + the action key (if any) it unlocks.
    let (status, status_style, action): (String, Style, Option<&str>) = if !can_update {
        (
            "self-update off for this build".into(),
            Style::default().fg(Color::DarkGray),
            None,
        )
    } else {
        match update {
            UpdateState::Idle => (
                "✓ up to date".into(),
                Style::default().fg(super::theme::ATTN),
                Some("c check"),
            ),
            UpdateState::Checking => (
                "checking for updates…".into(),
                Style::default().fg(Color::Gray),
                Some("c check"),
            ),
            UpdateState::Installing(v) => (
                format!("↻ downloading v{v}…"),
                Style::default().fg(Color::Gray),
                None,
            ),
            UpdateState::Ready(v) => (
                format!("↻ v{v} ready"),
                Style::default().fg(super::theme::ATTN).add_modifier(Modifier::BOLD),
                Some("u restart to update"),
            ),
        }
    };
    let hint = match action {
        Some(a) => format!("{a} · esc close"),
        None => "esc close".into(),
    };

    let bold = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let label = Style::default().fg(Color::DarkGray);
    let link = Style::default().fg(Color::Cyan);
    let lines: Vec<Line> = vec![
        Line::from(Span::styled(format!("mmux v{version}"), bold)),
        Line::from(Span::styled(
            "persistent terminals for your coding agents",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("built by  ", label),
            Span::styled("marvinvr.ch", link),
        ]),
        Line::from(vec![
            Span::styled("source    ", label),
            Span::styled("github.com/marvinvr/mmux", link),
        ]),
        Line::from(""),
        Line::from(Span::styled(status, status_style)),
        Line::from(""),
        Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))),
    ];

    let widest = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let w = (widest as u16 + 4).clamp(24, 64);
    let h = lines.len() as u16 + 2;
    let rect = centered(area, w, h);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" About mmux ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The "Link another project" browser: a path header, a live filter, the current
/// directory's sub-folders (each tagged git / mmux.yaml / linked), a short preview of
/// the highlighted directory, and a key hint pinned to the bottom row.
fn render_linkbrowse(f: &mut Frame, area: Rect, b: &LinkBrowser) {
    let w = area.width.saturating_sub(6).clamp(30, 86);
    let h = area.height.saturating_sub(4).clamp(12, 24);
    let rect = centered(area, w, h);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Link a project ")
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height < 8 {
        return;
    }
    let width = inner.width as usize;

    // Rows: path header, filter, the list, a 3-line preview, and the hint.
    let path_y = inner.y;
    let query_y = inner.y + 1;
    let hint_y = inner.y + inner.height - 1;
    let prev_h = 3u16;
    let prev_y = hint_y - prev_h;
    let list_y = query_y + 1;
    let list_h = prev_y.saturating_sub(list_y) as usize;

    // Header: the directory being browsed.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("in ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                b.cwd_label(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect { x: inner.x, y: path_y, width: inner.width, height: 1 },
    );
    // Filter line.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Magenta)),
            Span::styled(b.query.clone(), Style::default().fg(Color::White)),
        ])),
        Rect { x: inner.x, y: query_y, width: inner.width, height: 1 },
    );

    // The directory list, scrolled to keep the selection on screen.
    let sel = b.sel();
    let scroll = if list_h > 0 && sel >= list_h { sel + 1 - list_h } else { 0 };
    let mut lines: Vec<Line> = Vec::new();
    if b.count() == 0 {
        lines.push(Line::from(Span::styled(
            "  (no sub-directories here — ← to go up)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for row in scroll..(scroll + list_h) {
        let Some(e) = b.entry_at(row) else { break };
        lines.push(linkrow(e, row == sel, inner.width));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect { x: inner.x, y: list_y, width: inner.width, height: list_h as u16 },
    );

    // A short preview of the highlighted directory (or an error in its place).
    let prev = match &b.error {
        Some(e) => vec![Line::from(Span::styled(format!("⚠ {e}"), Style::default().fg(Color::Red)))],
        None => preview_lines(b.preview.as_ref()),
    };
    f.render_widget(
        Paragraph::new(prev),
        Rect { x: inner.x, y: prev_y, width: inner.width, height: prev_h },
    );

    // Hint footer.
    let hint = format!("{} dirs · ↑↓ move · → enter · ← up · ⏎ link · esc cancel", b.count());
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_middle(&hint, width),
            Style::default().fg(Color::DarkGray),
        ))),
        Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 },
    );

    // Park the cursor at the end of the filter text.
    let cx = inner.x + 2 + (b.query.chars().count() as u16).min(inner.width.saturating_sub(3));
    f.set_cursor_position((cx, query_y));
}

/// One directory row in the link browser: a selection bar, the `name/`, and
/// right-aligned tags (linked / git / mmux.yaml).
fn linkrow(e: &DirEntry, selected: bool, width: u16) -> Line<'static> {
    let bar = if selected { "▌ " } else { "  " };
    let mut tags: Vec<&str> = Vec::new();
    if e.already {
        tags.push("linked");
    } else {
        if e.is_repo {
            tags.push("git");
        }
        if e.has_config {
            tags.push("mmux.yaml");
        }
    }
    let tag = if tags.is_empty() { String::new() } else { format!("[{}]", tags.join(" · ")) };
    let name_color = if e.already {
        Color::DarkGray
    } else if e.has_config {
        Color::Green
    } else {
        Color::Blue
    };

    let w = width as usize;
    let tagw = tag.chars().count();
    let label = truncate_middle(&format!("{bar}{}/", e.name), w.saturating_sub(tagw + 1));
    let pad = w.saturating_sub(label.chars().count() + tagw);
    let mut line = Line::from(vec![
        Span::styled(label, Style::default().fg(name_color)),
        Span::raw(" ".repeat(pad)),
        Span::styled(tag, Style::default().fg(Color::DarkGray)),
    ]);
    if selected {
        let p = width.saturating_sub(line.width() as u16);
        if p > 0 {
            line.spans.push(Span::raw(" ".repeat(p as usize)));
        }
        line.style = Style::default().bg(Color::Rgb(45, 45, 60));
    }
    line
}

/// The 3-line preview block for the highlighted directory: the path it would be linked
/// as, its git branch, and its config presence.
fn preview_lines(p: Option<&Preview>) -> Vec<Line<'static>> {
    let Some(p) = p else {
        return vec![Line::from(Span::styled(
            "—",
            Style::default().fg(Color::DarkGray),
        ))];
    };
    let dim = Style::default().fg(Color::DarkGray);
    let mut link_spans = vec![
        Span::styled("link as  ", dim),
        Span::styled(p.rel.clone(), Style::default().fg(Color::White)),
    ];
    if p.already {
        link_spans.push(Span::styled("  (already in workspace)", Style::default().fg(Color::Yellow)));
    }
    let git = match &p.branch {
        Some(branch) => Span::styled(branch.clone(), Style::default().fg(Color::Magenta)),
        None => Span::styled("not a git repo", dim),
    };
    let config = match (p.has_config, &p.name) {
        (true, Some(name)) => {
            Span::styled(format!("mmux.yaml · {name}"), Style::default().fg(Color::Green))
        }
        _ => Span::styled("no mmux.yaml (the global config applies)", dim),
    };
    vec![
        Line::from(link_spans),
        Line::from(vec![Span::styled("git      ", dim), git]),
        Line::from(vec![Span::styled("config   ", dim), config]),
    ]
}

/// The "+ New Process" guided form: a "Step N of 4" header, the fields already
/// entered (dim, for context), the active input or the review screen, an optional
/// validation warning, and a key hint pinned to the bottom row.
fn render_procform(f: &mut Frame, area: Rect, form: &ProcForm) {
    let w = area.width.saturating_sub(6).clamp(34, 72);
    let h = 12u16.min(area.height);
    let rect = centered(area, w, h);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" New process ")
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height < 3 {
        return;
    }

    let (label, hint) = match form.step {
        Step::Name => ("Name", "⏎ next · esc cancel"),
        Step::Command => ("Command", "⏎ next · esc cancel"),
        Step::Cwd => ("Working dir", "⏎ next (blank ok) · esc cancel"),
        Step::Review => ("Review", "←→ autostart · ⏎ create · esc cancel"),
    };

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("Step {} of {STEPS} · {label}", form.step_index()),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // The screen row the text cursor should sit on (set on text steps only).
    let mut input_y: Option<u16> = None;
    match form.step {
        Step::Review => {
            lines.push(field_line("Name", &form.name));
            lines.push(field_line("Command", &form.command));
            let cwd = if form.cwd.trim().is_empty() { "(project root)" } else { form.cwd.trim() };
            lines.push(field_line("Working dir", cwd));
            let mark = if form.autostart { " yes " } else { " no " };
            lines.push(Line::from(vec![
                Span::styled("Autostart    ", Style::default().fg(Color::Gray)),
                Span::styled(
                    mark,
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        step => {
            // Echo the fields gathered so far for context.
            if step != Step::Name {
                lines.push(field_line("Name", &form.name));
            }
            if step == Step::Cwd {
                lines.push(field_line("Command", &form.command));
            }
            let prompt = match step {
                Step::Name => "A label for the sidebar, e.g. Dev server",
                Step::Command => "The command to run, e.g. npm run dev",
                Step::Cwd => "Directory relative to the project (blank = project root)",
                Step::Review => "",
            };
            lines.push(Line::from(Span::styled(prompt, Style::default().fg(Color::DarkGray))));
            input_y = Some(inner.y + lines.len() as u16);
            lines.push(Line::from(vec![
                Span::styled("> ", Style::default().fg(Color::Magenta)),
                Span::styled(form.buf.clone(), Style::default().fg(Color::White)),
            ]));
        }
    }
    if let Some(err) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("⚠ {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    // Body above, hint pinned to the last inner row.
    let body = Rect { x: inner.x, y: inner.y, width: inner.width, height: inner.height - 1 };
    f.render_widget(Paragraph::new(lines), body);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)))),
        Rect { x: inner.x, y: inner.y + inner.height - 1, width: inner.width, height: 1 },
    );

    if let Some(y) = input_y {
        let cx = inner.x + 2 + (form.buf.chars().count() as u16).min(inner.width.saturating_sub(3));
        f.set_cursor_position((cx, y));
    }
}

/// A dim `label: value` context line in the process form.
fn field_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::DarkGray)),
        Span::styled(value.to_string(), Style::default().fg(Color::Gray)),
    ])
}

/// The Ctrl+P fuzzy file picker: a query line, the ranked match list (the
/// selection scrolled into view and highlighted), and a hint/count footer.
fn render_picker(f: &mut Frame, area: Rect, p: &Picker) {
    let w = area.width.saturating_sub(6).clamp(24, 90);
    let h = area.height.saturating_sub(4).clamp(6, 20);
    let rect = centered(area, w, h);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Open file ")
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height < 3 {
        return;
    }

    // Rows: query line on top, hint on the bottom, the list between.
    let query_y = inner.y;
    let hint_y = inner.y + inner.height - 1;
    let list_y = inner.y + 1;
    let list_h = hint_y.saturating_sub(list_y) as usize;
    let width = inner.width as usize;

    // Query line: `> typed query`.
    let query = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Magenta)),
        Span::styled(p.query.clone(), Style::default().fg(Color::White)),
    ]);
    f.render_widget(
        Paragraph::new(query),
        Rect { x: inner.x, y: query_y, width: inner.width, height: 1 },
    );

    // Scroll the list so the selection stays on screen.
    let sel = p.sel();
    let scroll = if list_h > 0 && sel >= list_h { sel + 1 - list_h } else { 0 };
    let mut lines: Vec<Line> = Vec::new();
    for row in scroll..(scroll + list_h) {
        let Some(path) = p.path_at(row) else { break };
        let selected = row == sel;
        // When too long, elide the middle so both the leading dirs and the
        // filename/extension stay visible.
        let mut text = truncate_middle(path, width);
        let style = if selected {
            // Pad to the full width so the highlight bar spans the row.
            let pad = width.saturating_sub(text.chars().count());
            text.push_str(&" ".repeat(pad));
            Style::default().fg(Color::Black).bg(Color::Magenta)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect { x: inner.x, y: list_y, width: inner.width, height: list_h as u16 },
    );

    // Footer: match count + the key hints.
    let hint = format!(
        "{} matches · ↑↓ move · ⏎ open · esc cancel",
        p.match_count()
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)))),
        Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 },
    );

    // Park the cursor at the end of the query (after the `> ` prompt).
    let cx = inner.x + 2 + (p.query.chars().count() as u16).min(inner.width.saturating_sub(3));
    f.set_cursor_position((cx, query_y));
}

/// A destructive-action confirmation: the question, then `y discard · n cancel`. Red
/// border to signal it can't be undone.
/// A yes/no modal. The body may span several lines (split on `\n`); the box sizes
/// itself to the widest line and the per-action `hint` sits below a blank spacer.
fn render_confirm(f: &mut Frame, area: Rect, title: &str, body: &str, hint: &str) {
    let body_lines: Vec<&str> = body.lines().collect();
    let widest = body_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(hint.chars().count());
    let w = (widest as u16 + 4).clamp(20, 64);
    // Body lines + a blank spacer + the hint, wrapped in the two borders.
    let h = body_lines.len() as u16 + 4;
    let rect = centered(area, w, h);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines: Vec<Line> = body_lines
        .iter()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::White))))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        hint.to_string(),
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_prompt(f: &mut Frame, area: Rect, title: &str, buf: &str, kind: PromptKind) {
    let w = area.width.saturating_sub(8).min(60).max(20);
    let rect = centered(area, w, 4);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let hint = match kind {
        PromptKind::Commit => "⏎ commit · esc cancel",
        PromptKind::NewBranch => "⏎ create & switch · esc cancel",
    };
    let lines = vec![
        Line::from(Span::styled(buf.to_string(), Style::default().fg(Color::White))),
        Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines), inner);
    // Park the real cursor at the end of the typed text.
    let cx = inner.x + (buf.chars().count() as u16).min(inner.width.saturating_sub(1));
    f.set_cursor_position((cx, inner.y));
}

/// A `w`×`h` rectangle centered inside `area` (clamped to fit).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// Trim `s` from the left to fit `max` columns, prefixing `…` so the useful tail
/// (filename / branch leaf) stays visible in a narrow column.
fn truncate_left(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if max == 0 || n <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - max + 1).collect();
    format!("…{tail}")
}

/// Trim `s` to fit `max` columns by eliding the *middle* with `…`, keeping the
/// head and the tail so both the start of the name and its end (the file
/// extension) stay visible. The tail keeps the spare column so the extension
/// survives one-character-tighter fits.
fn truncate_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if max == 0 || n <= max {
        return s.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let budget = max - 1; // one column for the ellipsis
    let tail = budget.div_ceil(2); // tail gets the spare column → keeps the extension
    let head = budget - tail;
    let head_str: String = s.chars().take(head).collect();
    let tail_str: String = s.chars().skip(n - tail).collect();
    format!("{head_str}…{tail_str}")
}
