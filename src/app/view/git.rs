//! Rendering for the native git panel.
//!
//! The right column is three stacked, individually-bordered boxes — **Changes**
//! (top, flexible, rendered as a compressed directory *tree* via
//! [`git::tree_rows`](crate::git::tree_rows)), **Branches** (middle), **Commits**
//! (bottom, a scrollable, selectable history). These are free functions
//! over `&GitPanel` so they stay decoupled from the rest of `App`; the thin
//! wrapper that owns the per-frame hit [`Regions`](super::Regions) lives in
//! [`super::pane`] and stores the row maps [`render_git`] returns. The modal
//! overlays that float over the whole UI are rendered by [`super::overlay`].

use crate::app::git::{GitPanel, Section};
use crate::git::{Branch, Commit, FileEntry, Stage, TreeRow};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// The clickable rows the panel exposes this frame, for mouse routing.
#[derive(Default)]
pub(crate) struct GitRows {
    pub rows: Vec<(u16, usize)>,     // tree rows: screen y → cursor (tree-row) index
    pub branches: Vec<(u16, usize)>, // branch rows: screen y → branch index
    pub commits: Vec<(u16, usize)>,  // commit rows: screen y → log index
    pub changes_area: Option<Rect>,  // the Changes box, so a whitespace click focuses it
    pub branches_area: Option<Rect>, // the Branches box, ditto
    pub commits_area: Option<Rect>,  // the Commits box, ditto
}

/// Draw the git panel into `area` as three bordered boxes, returning the file, branch
/// and commit row maps for click routing.
pub(crate) fn render_git(f: &mut Frame, area: Rect, git: &GitPanel, focused: bool) -> GitRows {
    let mut hit = GitRows::default();
    if area.width < 3 || area.height < 3 {
        return hit;
    }

    // Branches + Commits are equal-height side boxes; Changes takes the rest. On a
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
    render_changes(f, chunks[0], git, focused, &mut hit.rows);
    if side_h > 0 {
        hit.branches_area = Some(chunks[1]);
        render_branches(f, chunks[1], git, focused, &mut hit.branches);
        hit.commits_area = Some(chunks[2]);
        render_commits(f, chunks[2], git, focused, &mut hit.commits);
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

/// The shared skeleton of the three panel boxes (Changes / Branches / Commits): draw the
/// bordered box, bail if it collapsed, show `empty` when the collection is empty, else
/// window over `len` items around `cursor` so the selection stays on screen — building
/// each visible row via `row` and registering its screen `y` in `hit` for click routing.
fn render_box<F>(
    f: &mut Frame,
    area: Rect,
    title: String,
    active: bool,
    len: usize,
    cursor: usize,
    empty: &str,
    hit: &mut Vec<(u16, usize)>,
    mut row: F,
) where
    F: FnMut(usize, bool, u16) -> Line<'static>,
{
    let block = boxed(title, active);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if len == 0 {
        f.render_widget(dim_line(empty), inner);
        return;
    }
    let (start, count) = window(len, cursor, inner.height as usize);
    let mut lines = Vec::with_capacity(count);
    for off in 0..count {
        let i = start + off;
        lines.push(row(i, active && i == cursor, inner.width));
        hit.push((inner.y + off as u16, i));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_changes(
    f: &mut Frame,
    area: Rect,
    git: &GitPanel,
    focused: bool,
    hit: &mut Vec<(u16, usize)>,
) {
    let active = focused && git.section == Section::Changes;
    let mut title = String::from(" Changes ");
    if let Some(b) = git.busy {
        title.push_str(&format!("· {b} "));
    }
    // Every tree row (dir / file) is selectable, so every visible one is registered.
    render_box(f, area, title, active, git.rows.len(), git.cursor, "  working tree clean", hit, |i, selected, w| {
        match &git.rows[i] {
            TreeRow::Dir { label, depth, staged, .. } => node_row(label, *depth, *staged, selected, w),
            TreeRow::File { idx, depth } => file_row(&git.files[*idx], *depth, selected, w),
        }
    });
}

fn render_branches(
    f: &mut Frame,
    area: Rect,
    git: &GitPanel,
    focused: bool,
    hit: &mut Vec<(u16, usize)>,
) {
    let active = focused && git.section == Section::Branches;
    render_box(f, area, " Branches ".into(), active, git.branches.len(), git.branch_cursor, "  no branches", hit, |i, selected, w| {
        branch_row(&git.branches[i], selected, w)
    });
}

fn render_commits(
    f: &mut Frame,
    area: Rect,
    git: &GitPanel,
    focused: bool,
    hit: &mut Vec<(u16, usize)>,
) {
    let active = focused && git.section == Section::Commits;
    render_box(f, area, " Commits ".into(), active, git.log.len(), git.commit_cursor, "  no commits", hit, |i, selected, w| {
        commit_row(&git.log[i], selected, w)
    });
}

/// Pad a selected row with trailing spaces and paint the highlight background so the
/// selection bar spans the full column width (matching the sidebar's `entry_line`),
/// rather than stopping at the end of the name.
fn fill_selection(line: &mut Line<'static>, width: u16) {
    super::theme::fill_row_bg(line, width, super::theme::SELECTION_BG);
}

/// A directory row: selection bar, indentation, an aggregate staging checkbox, then the
/// (possibly chain-compressed) name with a trailing slash.
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
        return Color::Green; // brand-new file = an addition
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

fn commit_row(c: &Commit, selected: bool, width: u16) -> Line<'static> {
    let bar = if selected { "▌" } else { " " };
    // bar + short hash + space; the subject fills whatever's left.
    let chrome = 1 + c.short.chars().count() + 1;
    let subject = truncate_middle(&c.summary, (width as usize).saturating_sub(chrome));
    let mut line = Line::from(vec![
        Span::styled(bar.to_string(), Style::default().fg(Color::Magenta)),
        Span::styled(format!("{} ", c.short), Style::default().fg(Color::Yellow)),
        Span::styled(subject, Style::default().fg(Color::Gray)),
    ]);
    if selected {
        fill_selection(&mut line, width);
    }
    line
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
/// survives one-character-tighter fits. Shared with the modal renderers in
/// [`super::overlay`].
pub(crate) fn truncate_middle(s: &str, max: usize) -> String {
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
