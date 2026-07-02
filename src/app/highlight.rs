//! Syntax highlighting for the git diff preview.
//!
//! Driven by [`syntect`] on its pure-Rust `fancy-regex` backend (no oniguruma C dep, so
//! the release cross-builds stay simple). The heavy syntax + theme dumps deserialize
//! once, lazily, on the first diff opened, then are reused for the process lifetime.
//!
//! Highlighting is **per line, stateless**: a diff hunk isn't contiguous source (there
//! are gaps between hunks, and `-`/`+` lines interleave the old and new file), so
//! carrying parser state line-to-line would corrupt more than it helps. Each line is
//! coloured on its own — multi-line constructs (block comments, multi-line strings)
//! aren't tracked, the accepted trade-off for a view that never sees the whole file.
//! Colours come back as ratatui RGB foregrounds; the diff renderer lays its own line
//! backgrounds and gutter on top (see [`view::pane`](super::view)).

use ratatui::style::Color;
use std::path::Path;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// Foreground for text we can't (or chose not to) highlight — a soft off-white that
/// reads on both the tinted and the plain diff rows.
pub(crate) const PLAIN: Color = Color::Rgb(201, 209, 217);

/// The lazily-loaded syntax and theme sets, shared across every diff for the run.
struct Assets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        // A muted dark theme whose foregrounds sit legibly over our subtle add/del tints.
        let theme = ThemeSet::load_defaults().themes["base16-ocean.dark"].clone();
        Assets { syntaxes, theme }
    })
}

/// A per-file highlighter: it resolves a syntax from the path's extension once, then
/// colours code lines one at a time.
pub(crate) struct Highlighter {
    syntax: Option<&'static SyntaxReference>,
}

impl Highlighter {
    /// Pick a syntax by file extension. Unknown or extensionless files highlight as
    /// [`plain`](Self::plain).
    pub(crate) fn for_path(path: &str) -> Self {
        let ss = &assets().syntaxes;
        let syntax = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| ss.find_syntax_by_extension(e));
        Highlighter { syntax }
    }

    /// A highlighter that never colours — for oversized diffs where lighting every line
    /// isn't worth the parse (see the guard in `DiffView::build`).
    pub(crate) fn plain() -> Self {
        Highlighter { syntax: None }
    }

    /// Colour one line of code (the leading diff sign already stripped) into RGB
    /// segments. Falls back to a single [`PLAIN`] segment when there's no syntax for the
    /// file or parsing fails.
    pub(crate) fn line(&self, code: &str) -> Vec<(Color, String)> {
        let Some(syntax) = self.syntax else {
            return vec![(PLAIN, code.to_string())];
        };
        let a = assets();
        let mut h = HighlightLines::new(syntax, &a.theme);
        match h.highlight_line(code, &a.syntaxes) {
            Ok(ranges) => ranges
                .into_iter()
                .map(|(style, text)| {
                    let c = style.foreground;
                    (Color::Rgb(c.r, c.g, c.b), text.to_string())
                })
                .collect(),
            Err(_) => vec![(PLAIN, code.to_string())],
        }
    }
}
