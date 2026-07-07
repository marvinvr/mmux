//! Comment-preserving raw-text YAML editing for mmux config files.
//!
//! The in-TUI forms (new/edit process, link project) and the agent manager write
//! back to the user's `mmux.yaml`/global config by splicing the *raw text* rather
//! than round-tripping through serde — a serde re-serialize would strip every
//! comment and reflow the file. Everything here is pure string surgery over the
//! typed [`ProcessDraft`]/[`AgentDraft`] the caller hands in; the typed schema,
//! load, and merge live in the parent [`super`] module.

use super::{global_config_path, load_file, AgentDef, AgentDraft, ProcessDraft};
use anyhow::{Context, Result};
use std::path::Path;

// ── shared onboarding text ───────────────────────────────────────────────────
// The first-config comment blocks below are pushed verbatim by the scaffolds here
// AND by the `mmux init` wizard (`crate::wizard`, via the `crate::config` re-exports).
// They were duplicated in both writers and had drifted; keeping the byte-identical
// pieces in one place is what stops that. Each carries its own trailing newline(s),
// so a site just pushes the const. (The richer STARTER template and the wizard's
// live-value headers stay bespoke — they are NOT byte-identical, by design.)

/// Intro lines shared by the project scaffold and the wizard's project file.
pub(crate) const PROJECT_HEADER: &str = "# mmux workspace config.\n\
    # Run `mmux` in this directory to open (or reattach to) the session.\n\
    # New here? Run `mmux docs` for the full guide, or visit https://mmux.org.\n";

/// The `# Agents:` intro line (the `agents:` block or its example follows).
pub(crate) const PROJECT_AGENTS_COMMENT: &str =
    "# Agents: interactive programs you spawn on demand from the sidebar.\n";

/// The commented-out `agents:` example, shown when no agents are written live.
pub(crate) const PROJECT_AGENTS_EXAMPLE: &str = "# agents:\n\
    #   - name: Claude\n\
    #     cmd: claude\n\
    #     args: [\"--dangerously-skip-permissions\"]\n\n";

/// The `# Processes:` explanatory comment (the block or its example follows).
pub(crate) const PROJECT_PROCESSES_COMMENT: &str =
    "# Processes: commands you start/stop and watch. cwd is relative to this file.\n\
    # An optional `stop:` shell line (e.g. docker compose down) runs in that dir when\n\
    # the process is stopped or mmux quits — handy for tearing down what it started.\n";

/// The commented-out `processes:` example.
pub(crate) const PROJECT_PROCESSES_EXAMPLE: &str = "# processes:\n\
    #   - name: Dev server\n\
    #     cmd: npm\n\
    #     args: [\"run\", \"dev\"]\n\
    #     autostart: false\n\
    #     # stop: docker compose down\n\n";

/// The `# Linked projects:` explanatory comment (the block or its example follows).
pub(crate) const PROJECT_LINKED_COMMENT: &str =
    "# Linked projects: other projects to show alongside this one in the same\n\
    # workspace — any directories you want grouped together (extra clones, a\n\
    # related repo, a service), each its own sidebar group. One level deep,\n\
    # de-duplicated by path.\n";

/// The commented-out `linked-projects:` example.
pub(crate) const PROJECT_LINKED_EXAMPLE: &str = "# linked-projects:\n\
    #   - ../myproject2\n";

/// Header for a fresh global config (`~/.mmux/config.yaml`); the `agents:` block follows.
pub(crate) const GLOBAL_HEADER: &str = "# mmux global config (~/.mmux/config.yaml).\n\
    # Agents here are available in EVERY project. A project's mmux.yaml can\n\
    # override or add to them by name.\n\
    # Full guide: run `mmux docs`, or visit https://mmux.org.\n\n";

/// Trailing git-panel hint for a global config (follows the agents block).
pub(crate) const GLOBAL_GIT_PANEL_HINT: &str = "\n\
    # A git panel is shown automatically in every git repo. To disable it:\n\
    # git-panel:\n\
    #   enabled: false\n";

/// The agents declared in the global config (`~/.mmux/config.yaml`), or empty when
/// there's no global config. The in-TUI agent manager reads this to seed its rows,
/// since it edits the global file specifically (a project's agents merge on top).
pub fn global_agents() -> Vec<AgentDef> {
    load_file(global_config_path().as_deref())
        .ok()
        .flatten()
        .map(|c| c.agents)
        .unwrap_or_default()
}

/// Double-quote a command-line token when a bare word wouldn't survive [`shell_split`]
/// (it holds whitespace, or is empty). No escaping — enough to round-trip typed input.
pub(crate) fn quote_token(s: &str) -> String {
    if s.is_empty() || s.contains(char::is_whitespace) {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

/// Append `p` to the `processes:` list in `path`, preserving the file's existing
/// comments and layout — we edit the raw text rather than round-tripping through
/// serde (which would strip every comment). Creates the file/block if absent.
pub fn append_process(path: &Path, p: &ProcessDraft) -> Result<()> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let updated = if original.trim().is_empty() {
        // Brand-new (or empty) file: don't leave it as a bare `processes:` block —
        // write the documented scaffold (header, `mmux docs` pointer, commented example
        // sections) with this process live, matching what `mmux init` produces. Existing
        // files are spliced in place so their comments/layout survive.
        scaffold_project_file(&render_item(p, 2), "")
    } else {
        insert_process(&original, p)?
    };
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Replace the `processes:` item named `name` in `path` with `p`, preserving the
/// file's surrounding comments and layout (the edited item is re-rendered, so any
/// comments *inside* that one entry are dropped). Errors if the item can't be found —
/// e.g. its `name:` is written in a shape the raw-text scan doesn't recognise.
pub fn replace_process(path: &Path, name: &str, p: &ProcessDraft) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let updated = replace_named_item(&original, "processes", name, p)?;
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Remove the `processes:` item named `name` from `path`, preserving the file's other
/// comments and layout. Errors if the item can't be found. Leaving `processes:` with
/// no items is fine — it parses back to an empty list.
pub fn remove_process(path: &Path, name: &str) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let updated = delete_named_item(&original, "processes", name)?;
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Rewrite the top-level `agents:` block in `path` to exactly `agents`, preserving
/// everything else in the file (its header comments, the git-panel example, any other
/// blocks). The in-TUI agent manager always targets the global config, so a
/// missing/blank file is seeded with the documented global scaffold and a file with no
/// `agents:` block gains one appended at the end. Unlike the process editors this
/// replaces the *whole* block (the manager owns the full list), so any hand-written
/// comments *inside* the old block are dropped — the surrounding file is untouched.
pub fn write_agents(path: &Path, agents: &[AgentDraft]) -> Result<()> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let updated = if original.trim().is_empty() {
        scaffold_global_file(&render_agents_block(agents))
    } else {
        replace_agents_block(&original, agents)
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// The `agents:` block for `agents` — the key line plus one item each, or the empty
/// placeholder `agents: []` when none are enabled (which parses back to an empty list).
fn render_agents_block(agents: &[AgentDraft]) -> String {
    if agents.is_empty() {
        return "agents: []\n".to_string();
    }
    let mut s = String::from("agents:\n");
    for a in agents {
        s.push_str(&render_agent_item(a, 2));
    }
    s
}

/// Render one `agents:` list item at the given indent, matching the hand-written style
/// (unquoted scalars where safe, quoted args). `args` is always emitted — even `[]` —
/// so an agent toggled out of danger mode reads clearly as "no flags".
pub(crate) fn render_agent_item(a: &AgentDraft, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let sub = " ".repeat(indent + 2);
    let mut s = format!("{ind}- name: {}\n", yaml_scalar(&a.name));
    s.push_str(&format!("{sub}cmd: {}\n", yaml_scalar(&a.cmd)));
    s.push_str(&format!("{sub}args: {}\n", yaml_args(&a.args)));
    s
}

/// Swap the existing top-level `agents:` block for a freshly rendered one, or append a
/// new block at EOF when there's none. Preserves every line outside the block (kept
/// pure for testing). The block's item lines are `k..block_end`, so any trailing blank
/// lines/comments after the last item — the git-panel example the scaffold writes —
/// survive as the file's tail.
fn replace_agents_block(text: &str, agents: &[AgentDraft]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let block = render_agents_block(agents);
    let Some(k) = lines.iter().position(|l| top_level_key(l) == Some("agents")) else {
        // No block yet: append a fresh one (with a blank separator) at EOF.
        return append_block(text, &block);
    };
    // Replace the whole block (`agents:` line through its last item) with the fresh one.
    splice_lines(&lines, k, block_end(&lines, k), &block)
}

/// A fresh, self-documenting global config (`~/.mmux/config.yaml`) seeded with
/// `agents_block`. Matches the header the `mmux init` wizard writes for a first-run
/// global file, so a config born from the in-TUI agent manager explains itself.
fn scaffold_global_file(agents_block: &str) -> String {
    let mut s = String::new();
    s.push_str(GLOBAL_HEADER);
    s.push_str(agents_block);
    s.push_str(GLOBAL_GIT_PANEL_HINT);
    s
}

/// Append `rel` (a path, relative to `path`'s directory) to the `linked-projects:`
/// list in `path`, preserving the file's comments and layout. Creates the file/block
/// if absent. Mirrors [`append_process`]; used by the in-TUI "Link another project"
/// browser so a linked sibling survives the next reopen.
pub fn append_linked_project(path: &Path, rel: &str) -> Result<()> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let updated = if original.trim().is_empty() {
        // Same as [`append_process`]: seed an empty/absent file with the documented
        // scaffold rather than a bare `linked-projects:` block.
        scaffold_project_file("", &format!("  - {}\n", yaml_scalar(rel)))
    } else {
        insert_linked_project(&original, rel)?
    };
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// A fresh, self-documenting project file for a directory that has no config yet — the
/// same header, `mmux docs` pointer, and section comments the `mmux init` starter uses,
/// so a file born from the in-TUI "+ New Process" / "Link project" flows explains itself
/// instead of starting life as a bare block. `processes`/`linked` hold the already-
/// rendered live list items (indent 2) for whichever section triggered the creation; an
/// empty string leaves that section as a commented example.
fn scaffold_project_file(processes: &str, linked: &str) -> String {
    let mut s = String::new();
    s.push_str(PROJECT_HEADER);
    s.push_str("# `name` is optional — it defaults to this directory's name.\n");
    s.push_str("# name: my-workspace\n\n");

    s.push_str(PROJECT_AGENTS_COMMENT);
    s.push_str(PROJECT_AGENTS_EXAMPLE);

    s.push_str(PROJECT_PROCESSES_COMMENT);
    if processes.is_empty() {
        s.push_str(PROJECT_PROCESSES_EXAMPLE);
    } else {
        s.push_str("processes:\n");
        s.push_str(processes);
        s.push('\n');
    }

    s.push_str(PROJECT_LINKED_COMMENT);
    if linked.is_empty() {
        s.push_str(PROJECT_LINKED_EXAMPLE);
    } else {
        s.push_str("linked-projects:\n");
        s.push_str(linked);
    }
    s
}

/// Rebuild `text` from its `lines`, dropping the range `start..end` and splicing
/// `replacement` in at `start`. `replacement` is emitted verbatim (it carries its own
/// trailing newline); every surviving line is re-emitted with a `\n`. `start == end`
/// inserts without removing anything; `start == lines.len()` appends at EOF. The one
/// line-splice primitive behind the in-place edit/replace/delete forms.
fn splice_lines(lines: &[&str], start: usize, end: usize, replacement: &str) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i == start {
            out.push_str(replacement);
        }
        if i >= start && i < end {
            continue; // drop the replaced range
        }
        out.push_str(l);
        out.push('\n');
    }
    if start >= lines.len() {
        out.push_str(replacement); // the loop never reached an EOF insertion point
    }
    out
}

/// Append a freshly rendered top-level `block_text` to `text` at EOF, separated from
/// existing content by a blank line. The shared "there's no such block yet" tail for
/// [`splice_block_item`] and [`replace_agents_block`]; `block_text` carries its own
/// trailing newline.
fn append_block(text: &str, block_text: &str) -> String {
    let mut out = text.trim_end_matches('\n').to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block_text);
    out
}

/// Splice a rendered process item into `text`'s top-level `processes:` block.
fn insert_process(text: &str, p: &ProcessDraft) -> Result<String> {
    splice_block_item(text, "processes", |indent| render_item(p, indent))
}

/// Splice a `- <path>` entry into `text`'s top-level `linked-projects:` block. Like
/// [`insert_process`] it edits the raw text (not a serde round-trip) so the file's
/// comments and layout survive.
fn insert_linked_project(text: &str, rel: &str) -> Result<String> {
    splice_block_item(text, "linked-projects", |indent| {
        format!("{}- {}\n", " ".repeat(indent), yaml_scalar(rel))
    })
}

/// Splice a rendered list item into `text`'s top-level `block:` sequence, preserving
/// the file's existing comments and layout (kept pure for testing). `render(indent)`
/// produces the item at the block's indentation. The item lands among any existing
/// entries — after the last one, before trailing blank lines/comments. With no block
/// it's created at EOF; an `[]`/`null` placeholder is replaced by the real list; an
/// inline value (`block: foo`) is refused, since appending lines can't extend it.
fn splice_block_item(text: &str, block: &str, render: impl Fn(usize) -> String) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(k) = lines.iter().position(|l| top_level_key(l) == Some(block)) else {
        // No block yet: append a fresh one (with a blank separator) at EOF.
        return Ok(append_block(text, &format!("{block}:\n{}", render(2))));
    };

    // An inline value other than an empty placeholder (`block: foo`) is a shape we
    // can't safely extend by appending lines — leave it to the user.
    let after = lines[k].splitn(2, ':').nth(1).map(str::trim).unwrap_or("");
    let empty_marker = matches!(after, "" | "[]" | "{}" | "~" | "null");
    if !empty_marker {
        anyhow::bail!("`{block}:` is written inline — add the entry by hand");
    }

    let item = render(block_item_indent(&lines, k).unwrap_or(2));
    if after.is_empty() {
        // A real block (bare `block:` with any existing items): splice the new item in
        // among them — after the last one, before any trailing blank lines/comments.
        let at = block_end(&lines, k);
        Ok(splice_lines(&lines, at, at, &item))
    } else {
        // Drop the `[]`/`null` placeholder line so the new item parses as the block's
        // value: rewrite `block:` and splice the item right after it (an inline
        // placeholder has no item lines of its own to preserve).
        Ok(splice_lines(&lines, k, k + 1, &format!("{block}:\n{item}")))
    }
}

/// The key name if `line` is a top-level mapping key (column 0, `key:` …), else
/// `None` — used to find the `processes:` block and detect where it ends.
fn top_level_key(line: &str) -> Option<&str> {
    if line.is_empty() || line.starts_with(char::is_whitespace) || !line.contains(':') {
        return None;
    }
    let key = line.splitn(2, ':').next()?.trim_end();
    if key.is_empty() || key.starts_with('#') || key.contains(char::is_whitespace) {
        return None;
    }
    Some(key)
}

/// Indentation (leading spaces) of the first `- ` list item under the block at `k`,
/// so a new item lines up with its siblings. `None` when the block is empty.
fn block_item_indent(lines: &[&str], k: usize) -> Option<usize> {
    for line in &lines[k + 1..] {
        if top_level_key(line).is_some() {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('-') {
            return Some(line.len() - trimmed.len());
        }
    }
    None
}

/// Line index to insert a new item at: just past the block's last real line (the
/// next top-level key or EOF), backed up over trailing blank lines and comments so
/// the entry sits with its siblings rather than below a trailing comment block.
fn block_end(lines: &[&str], k: usize) -> usize {
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(k + 1) {
        if top_level_key(line).is_some() {
            end = i;
            break;
        }
    }
    while end > k + 1 {
        let t = lines[end - 1].trim();
        if t.is_empty() || t.starts_with('#') {
            end -= 1;
        } else {
            break;
        }
    }
    end
}

/// Replace the `block:` list item named `name` in `text` with a freshly rendered `p`,
/// preserving the file's other comments and layout (kept pure for testing). Any blank
/// separator lines trailing the old item are kept, so the spacing between entries
/// survives. Errors if the named item can't be located.
fn replace_named_item(text: &str, block: &str, name: &str, p: &ProcessDraft) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, mut end, indent) = named_item_span(&lines, block, name)
        .ok_or_else(|| anyhow::anyhow!("couldn't find “{name}” under `{block}:` — edit it by hand"))?;
    // Don't consume the blank line(s) between this item and the next — re-emit them.
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    Ok(splice_lines(&lines, start, end, &render_item(p, indent)))
}

/// Delete the `block:` list item named `name` from `text`, preserving the file's other
/// comments and layout (kept pure for testing). Errors if it can't be located.
fn delete_named_item(text: &str, block: &str, name: &str) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end, _) = named_item_span(&lines, block, name)
        .ok_or_else(|| anyhow::anyhow!("couldn't find “{name}” under `{block}:` — edit it by hand"))?;
    Ok(splice_lines(&lines, start, end, ""))
}

/// Locate the list item under top-level `block:` whose `name:` equals `name`, returning
/// its `(start, end, item_indent)` — `start..end` is its line range (dash line through
/// the line before the next sibling dash / next top-level key / EOF). The counterpart to
/// [`splice_block_item`] for the in-place edit/delete forms; `None` if not found.
fn named_item_span(lines: &[&str], block: &str, name: &str) -> Option<(usize, usize, usize)> {
    let k = lines.iter().position(|l| top_level_key(l) == Some(block))?;
    let indent = block_item_indent(lines, k)?;
    // The block runs until the next top-level key (or EOF) — no comment back-up here.
    let region_end = lines
        .iter()
        .enumerate()
        .skip(k + 1)
        .find(|(_, l)| top_level_key(l).is_some())
        .map_or(lines.len(), |(i, _)| i);
    // Each `- ` at exactly the item indent starts a new entry; deeper dashes (a nested
    // block sequence like `args:`) belong to the current one.
    let starts: Vec<usize> = (k + 1..region_end)
        .filter(|&i| {
            let t = lines[i].trim_start();
            t.starts_with('-') && lines[i].len() - t.len() == indent
        })
        .collect();
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(region_end);
        if item_name(&lines[start..end]) == Some(name.to_string()) {
            return Some((start, end, indent));
        }
    }
    None
}

/// The `name:` value declared inside one list item's lines — on the dash line
/// (`- name: X`) or a following `name: X` line — unquoted, with an inline `# comment`
/// on an unquoted value stripped. `None` if the item has no `name:`.
fn item_name(item: &[&str]) -> Option<String> {
    for (i, line) in item.iter().enumerate() {
        // The dash line carries its first key after the `- `; later lines are plain keys.
        let content = if i == 0 {
            let t = line.trim_start();
            t.strip_prefix('-').map_or(t, str::trim_start)
        } else {
            line.trim_start()
        };
        if let Some(rest) = content.strip_prefix("name:") {
            let val = rest.trim();
            // Drop a trailing `# comment` on a bare scalar (YAML needs a space before #).
            let val = match val.starts_with(['"', '\'']) {
                true => val,
                false => val.find(" #").map_or(val, |j| val[..j].trim_end()),
            };
            return Some(unquote_scalar(val));
        }
    }
    None
}

/// Strip matching surrounding single/double quotes from a YAML scalar (no escape
/// processing — enough to read back a `name:` we or the user wrote).
fn unquote_scalar(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && *b.last().unwrap() == b[0] {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Render one `processes:` list item at the given indent, matching the hand-written
/// style (unquoted scalars where safe, quoted args). `args`/`cwd` are emitted only
/// when set, so a bare command stays minimal.
fn render_item(p: &ProcessDraft, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let sub = " ".repeat(indent + 2);
    let mut s = format!("{ind}- name: {}\n", yaml_scalar(&p.name));
    s.push_str(&format!("{sub}cmd: {}\n", yaml_scalar(&p.cmd)));
    if !p.args.is_empty() {
        s.push_str(&format!("{sub}args: {}\n", yaml_args(&p.args)));
    }
    if let Some(cwd) = &p.cwd {
        s.push_str(&format!("{sub}cwd: {}\n", yaml_scalar(cwd)));
    }
    if let Some(stop) = &p.stop {
        s.push_str(&format!("{sub}stop: {}\n", yaml_scalar(stop)));
    }
    s.push_str(&format!("{sub}autostart: {}\n", p.autostart));
    s
}

/// Render an argument list as a YAML flow sequence of double-quoted scalars.
/// JSON-style quoting (via `{:?}`) is valid YAML, so this stays correct for args
/// with spaces or quotes.
pub(crate) fn yaml_args(args: &[String]) -> String {
    let inner: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
    format!("[{}]", inner.join(", "))
}

/// A scalar value, quoted only when YAML would otherwise mis-parse it. Keeps the
/// common case (`cmd: cargo`, `cwd: .`) clean while staying safe for input
/// containing `:`, `#`, quotes, brackets, or an indicator first character. Shared
/// with the `mmux init` wizard so both writers emit identically-styled YAML.
pub(crate) fn yaml_scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s == s.trim()
        && !s.contains(['#', ':', '"', '\'', '[', ']', '{', '}', '\n'])
        && !s.starts_with(['-', '?', '&', '*', '!', '|', '>', '%', '@', '`', ',']);
    if plain {
        s.to_string()
    } else {
        format!("{s:?}")
    }
}

/// Tokenize a command line on whitespace, with single/double quotes grouping a run
/// (quotes are removed; no escape processing — enough for typed commands).
pub(crate) fn shell_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false; // distinguishes "" (a real empty token) from no token
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                started = true;
                while let Some(q) = chars.next() {
                    if q == c {
                        break;
                    }
                    cur.push(q);
                }
            }
            c if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            _ => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

pub fn write_starter(dir: &Path) -> Result<()> {
    let path = dir.join("mmux.yaml");
    if path.exists() {
        println!("{} already exists — leaving it alone.", path.display());
        return Ok(());
    }
    std::fs::write(&path, STARTER).with_context(|| format!("writing {}", path.display()))?;
    println!("Created {}. Edit it, then run `mmux`.", path.display());
    Ok(())
}

const STARTER: &str = r#"# mmux workspace config.
# Run `mmux` in this directory to open (or reattach to) the session.
# New here? Run `mmux docs` for the full guide, or visit https://mmux.org.
# `name` is optional — it defaults to this directory's name.
# name: my-workspace

# Agents: interactive programs you spawn on demand. Each "+ New <name>" in the
# sidebar launches a fresh instance; its sidebar subtitle shows the terminal
# title the program sets, and a red dot appears when it rings the bell.
# More harnesses ship as presets (Gemini, Amp, opencode, Grok) — add/remove them
# any time with `mmux agents` or the sidebar's `a` key (both edit your global config).
agents:
  - name: Claude
    cmd: claude
    args: ["--dangerously-skip-permissions"]
  - name: Codex
    cmd: codex
    args: ["--dangerously-bypass-approvals-and-sandbox"]

# Processes: defined commands you start/stop and watch. cwd is relative to this file.
# An optional `stop:` shell line (e.g. docker compose down) runs in that dir when the
# process is stopped or mmux quits — handy for tearing down what it started.
processes:
  - name: Dev server
    cmd: npm
    args: ["run", "dev"]
    autostart: false
    # stop: docker compose down

# Linked projects: other projects to show alongside this one in the same workspace —
# any directories you want grouped together (extra clones, a related repo, a service).
# Each gets its own group in the sidebar; switch with [ and ]. Listing is one level
# deep and de-duplicated by path, so you can drop this same config into every project
# (even one that lists itself) without it ever expanding recursively.
# linked-projects:
#   - ../myproject2
#   - ../myproject3

# Notifications: when a session rings the bell (or emits a notification escape of
# its own), mmux raises a native desktop popup. It's delivered as a terminal escape
# sequence, so it works locally AND over SSH — the popup appears on whatever machine
# your terminal runs on. On by default; this block just shows the knobs.
# notifications:
#   enabled: true
#   mechanism: osc9     # osc9 (iTerm2/kitty/ghostty/wezterm) · osc777 (foot/urxvt/ghostty) · bell · command
#   only_when_unfocused: true
#   throttle_secs: 5
#   # command: 'terminal-notifier -title "$MMUX_NOTIFY_TITLE" -message "$MMUX_NOTIFY_BODY"'
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn draft() -> ProcessDraft {
        ProcessDraft {
            name: "Dev server".into(),
            cmd: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            cwd: None,
            autostart: false,
            stop: None,
        }
    }

    #[test]
    fn inserts_among_existing_processes_at_their_indent() {
        let text = "name: demo\n\nprocesses:\n  - name: Check\n    cmd: cargo\n    args: [\"check\"]\n";
        let out = insert_process(text, &draft()).unwrap();
        // The existing entry survives untouched and the new one follows it, same indent
        // and unquoted-where-safe style.
        assert!(out.contains("  - name: Check"));
        assert!(out.contains("  - name: Dev server"));
        assert!(out.contains("    cmd: npm"));
        assert!(out.contains("    args: [\"run\", \"dev\"]"));
        assert!(out.contains("    autostart: false"));
        assert!(out.find("name: Check").unwrap() < out.find("name: Dev server").unwrap());
        // A parse-back proves the splice is valid YAML with both entries.
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 2);
        assert_eq!(cfg.processes[1].name, "Dev server");
        assert_eq!(cfg.processes[1].args, vec!["run", "dev"]);
    }

    #[test]
    fn inserts_above_a_trailing_comment_block() {
        // The new entry should land with its siblings, not below the trailing comments.
        let text = "processes:\n  - name: A\n    cmd: x\n\n  # optional extras below\n";
        let out = insert_process(text, &draft()).unwrap();
        assert!(out.find("Dev server").unwrap() < out.find("optional extras").unwrap());
    }

    #[test]
    fn appends_a_fresh_block_when_absent() {
        let out = insert_process("name: demo\n", &draft()).unwrap();
        assert!(out.contains("\nprocesses:\n  - name: Dev server"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 1);
    }

    #[test]
    fn scaffolds_a_documented_file_for_a_new_process() {
        // A process added to a directory with no config gets the full documented
        // scaffold (header + `mmux docs` pointer + commented example sections), not a
        // bare `processes:` block — the live process sits in the processes section while
        // agents/linked-projects stay as commented examples.
        let out = scaffold_project_file(&render_item(&draft(), 2), "");
        assert!(out.starts_with("# mmux workspace config."));
        assert!(out.contains("mmux docs"));
        assert!(out.contains("# agents:"));
        assert!(out.contains("processes:\n  - name: Dev server"));
        assert!(out.contains("# linked-projects:"));
        // The commented processes example is gone (the real block took its place) and
        // the whole thing parses back to exactly the one process.
        assert!(!out.contains("# processes:"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 1);
        assert_eq!(cfg.processes[0].name, "Dev server");
        assert!(cfg.linked_projects.is_empty());
    }

    #[test]
    fn scaffolds_a_documented_file_for_a_linked_project() {
        let out = scaffold_project_file("", "  - ../sibling\n");
        assert!(out.contains("mmux docs"));
        assert!(out.contains("linked-projects:\n  - ../sibling"));
        // Processes stays a commented example when only a link was added.
        assert!(out.contains("# processes:"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert!(cfg.processes.is_empty());
        assert_eq!(cfg.linked_projects, vec!["../sibling"]);
    }

    #[test]
    fn append_process_scaffolds_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("mmux-scaffold-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mmux.yaml");
        let _ = std::fs::remove_file(&path);
        append_process(&path, &draft()).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("mmux docs"));
        assert!(written.contains("processes:\n  - name: Dev server"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replaces_an_empty_list_placeholder() {
        let out = insert_process("processes: []\nname: demo\n", &draft()).unwrap();
        assert!(!out.contains("[]"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 1);
    }

    #[test]
    fn refuses_an_inline_processes_value() {
        assert!(insert_process("processes: something\n", &draft()).is_err());
    }

    #[test]
    fn optional_fields_are_emitted_only_when_set() {
        let mut d = draft();
        d.args.clear();
        d.cwd = Some("backend".into());
        d.stop = Some("docker compose down".into());
        d.autostart = true;
        let out = insert_process("", &d).unwrap();
        assert!(!out.contains("args:"));
        assert!(out.contains("cwd: backend"));
        assert!(out.contains("stop: docker compose down"));
        assert!(out.contains("autostart: true"));
        // The stop line round-trips back to the parsed config.
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes[0].stop.as_deref(), Some("docker compose down"));
    }

    #[test]
    fn stop_is_omitted_when_unset() {
        // A bare draft (no stop) writes no `stop:` line.
        let out = insert_process("", &draft()).unwrap();
        assert!(!out.contains("stop:"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert!(cfg.processes[0].stop.is_none());
    }

    #[test]
    fn replaces_a_named_process_in_place_keeping_siblings_and_comments() {
        let text = "# top\nprocesses:\n  - name: Check\n    cmd: cargo\n    args: [\"check\"]\n\n  - name: Dev server\n    cmd: old\n    autostart: false\n";
        let mut d = draft();
        d.cmd = "npm".into();
        let out = replace_named_item(text, "processes", "Dev server", &d).unwrap();
        // The other entry and the leading comment survive untouched…
        assert!(out.contains("# top"));
        assert!(out.contains("  - name: Check"));
        // …and the edited one now carries the new command.
        assert!(out.contains("    cmd: npm"));
        assert!(!out.contains("cmd: old"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 2);
        assert_eq!(cfg.processes[1].name, "Dev server");
        assert_eq!(cfg.processes[1].cmd, "npm");
    }

    #[test]
    fn replace_can_rename_the_matched_process() {
        let text = "processes:\n  - name: Old\n    cmd: x\n";
        let mut d = draft();
        d.name = "New".into();
        let out = replace_named_item(text, "processes", "Old", &d).unwrap();
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 1);
        assert_eq!(cfg.processes[0].name, "New");
    }

    #[test]
    fn removes_a_named_process_leaving_the_rest() {
        let text = "processes:\n  - name: A\n    cmd: x\n  - name: B\n    cmd: y\n";
        let out = delete_named_item(text, "processes", "A").unwrap();
        assert!(!out.contains("name: A"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.processes.len(), 1);
        assert_eq!(cfg.processes[0].name, "B");
    }

    #[test]
    fn removing_the_only_process_leaves_an_empty_block() {
        let out = delete_named_item("name: demo\nprocesses:\n  - name: A\n    cmd: x\n", "processes", "A").unwrap();
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert!(cfg.processes.is_empty());
        assert_eq!(cfg.name.as_deref(), Some("demo"));
    }

    #[test]
    fn edit_and_delete_error_on_an_unknown_process() {
        assert!(delete_named_item("processes:\n  - name: A\n    cmd: x\n", "processes", "Nope").is_err());
        assert!(replace_named_item("processes:\n  - name: A\n    cmd: x\n", "processes", "Nope", &draft()).is_err());
    }

    #[test]
    fn inserts_among_existing_linked_projects_at_their_indent() {
        let text = "name: demo\n\nlinked-projects:\n  - ../a\n";
        let out = insert_linked_project(text, "../b").unwrap();
        assert!(out.contains("  - ../a"));
        assert!(out.contains("  - ../b"));
        assert!(out.find("../a").unwrap() < out.find("../b").unwrap());
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.linked_projects, vec!["../a".to_string(), "../b".to_string()]);
    }

    #[test]
    fn appends_a_fresh_linked_projects_block_when_absent() {
        // A commented `# linked-projects:` example must NOT be treated as the block.
        let out = insert_linked_project("name: demo\n# linked-projects:\n", "../b").unwrap();
        assert!(out.contains("\nlinked-projects:\n  - ../b"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.linked_projects, vec!["../b".to_string()]);
    }

    #[test]
    fn replaces_an_empty_linked_projects_placeholder() {
        let out = insert_linked_project("linked-projects: []\n", "../b").unwrap();
        assert!(!out.contains("[]"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.linked_projects, vec!["../b".to_string()]);
    }

    // ── agent manager: rewriting the global `agents:` block ──────────────────
    fn ag(name: &str, cmd: &str, args: &[&str]) -> AgentDraft {
        AgentDraft {
            name: name.into(),
            cmd: cmd.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn replace_agents_block_swaps_the_whole_list_keeping_the_rest() {
        // The header comment before and the git-panel example after the block must
        // survive; the block itself is replaced by the new list.
        let text = "# mmux global config\n\nagents:\n  - name: Claude\n    cmd: claude\n    args: []\n\n# git-panel:\n#   enabled: false\n";
        let out = replace_agents_block(text, &[
            ag("Claude", "claude", &["--dangerously-skip-permissions"]),
            ag("Gemini", "gemini", &["--yolo"]),
        ]);
        assert!(out.starts_with("# mmux global config"));
        assert!(out.contains("# git-panel:"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        let names: Vec<&str> = cfg.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Claude", "Gemini"]);
        assert_eq!(cfg.agents[0].args, vec!["--dangerously-skip-permissions"]);
        assert_eq!(cfg.agents[1].args, vec!["--yolo"]);
    }

    #[test]
    fn replace_agents_block_appends_when_absent() {
        let out = replace_agents_block("name: global\n", &[ag("Codex", "codex", &[])]);
        assert!(out.contains("\nagents:\n  - name: Codex"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.agents.len(), 1);
        assert_eq!(cfg.name.as_deref(), Some("global"));
    }

    #[test]
    fn empty_agent_list_writes_an_empty_placeholder() {
        let out = replace_agents_block("agents:\n  - name: Claude\n    cmd: claude\n", &[]);
        assert!(out.contains("agents: []"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert!(cfg.agents.is_empty());
    }

    #[test]
    fn scaffolds_a_documented_global_file_for_a_first_agent() {
        let out = scaffold_global_file(&render_agents_block(&[ag("Claude", "claude", &["--dangerously-skip-permissions"])]));
        assert!(out.starts_with("# mmux global config"));
        assert!(out.contains("mmux docs"));
        assert!(out.contains("agents:\n  - name: Claude"));
        assert!(out.contains("# git-panel:"));
        let cfg: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(cfg.agents.len(), 1);
    }

    #[test]
    fn write_agents_scaffolds_a_missing_global_file() {
        let dir = std::env::temp_dir().join(format!("mmux-agents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        let _ = std::fs::remove_file(&path);
        write_agents(&path, &[ag("Claude", "claude", &["--dangerously-skip-permissions"])]).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("mmux docs"));
        assert!(written.contains("agents:\n  - name: Claude"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn yaml_scalar_quotes_only_when_needed() {
        assert_eq!(yaml_scalar("Dev server"), "Dev server");
        assert_eq!(yaml_scalar("."), ".");
        assert_eq!(yaml_scalar("../proj2"), "../proj2");
        assert_eq!(yaml_scalar("build:dev"), "\"build:dev\"");
        assert_eq!(yaml_scalar("- weird"), "\"- weird\"");
    }
}
