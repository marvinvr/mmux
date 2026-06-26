use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Upper bound on projects in one workspace (root + linked). A backstop so a
/// runaway `linked-projects` list can't explode the sidebar.
const MAX_PROJECTS: usize = 8;

/// A workspace config, loaded from `mmux.yaml` (or `mmux.yml`) in a directory.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Optional display name for the workspace (shown in the sidebar title).
    #[serde(default)]
    pub name: Option<String>,
    /// Agent templates — interactive programs you spawn on demand (Claude, Codex, …).
    #[serde(default)]
    pub agents: Vec<AgentDef>,
    /// Defined processes — long-running commands you start/stop/watch (servers, ansible, …).
    #[serde(default)]
    pub processes: Vec<ProcessDef>,
    /// The built-in git panel on the right (changed files, staging, commit, push/
    /// pull, branch switcher). Shown automatically when the project is a git repo;
    /// this just disables it. Its width follows the left sidebar. `None` ⇒ enabled.
    #[serde(default, rename = "git-panel")]
    pub git_panel: Option<GitPanelConfig>,
    /// Desktop-notification behaviour. Delivered as terminal escape sequences, so
    /// they reach the user's machine even over SSH. `None` ⇒ the built-in defaults
    /// (enabled); see [`NotifyConfig`].
    #[serde(default)]
    pub notifications: Option<NotifyConfig>,
    /// Sibling project directories to load alongside this one — e.g. extra clones
    /// (`../myproject2`). Each becomes its own group in the sidebar. Paths are
    /// relative to this config's dir. Honored only in the directory you launch mmux
    /// in: a linked project's own `linked-projects` is ignored, so a config shared
    /// across clones can never expand recursively. See [`Config::load_workspace`].
    #[serde(default, rename = "linked-projects")]
    pub linked_projects: Vec<String>,
    /// Background self-update (Homebrew installs only). `None`/unset ⇒ enabled; see
    /// [`AutoUpdateConfig`] and [`crate::update`].
    #[serde(default, rename = "auto-update")]
    pub auto_update: Option<AutoUpdateConfig>,
    /// The directory the config was loaded from. Relative `cwd`s resolve against this.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// A loaded workspace: the root project (the dir mmux was launched in) plus every
/// directory it links to, in load order. Always non-empty (`projects[0]` is root).
pub struct Workspace {
    pub projects: Vec<Config>,
    /// Non-fatal problems (a linked project that was missing, unreadable, or beyond
    /// the cap) to surface without aborting startup.
    pub warnings: Vec<String>,
}

/// Settings for the built-in git panel.
#[derive(Debug, Clone, Deserialize)]
pub struct GitPanelConfig {
    /// Show the panel (default: true, whenever the project is a git repo). Its width
    /// always matches the left sidebar's, so there's no width knob.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Settings for background self-update (only acts on Homebrew installs). When enabled
/// and mmux was installed via brew, it checks for a newer release on startup and once a
/// day, installs it in the background, and shows a quiet "restart to update" badge.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoUpdateConfig {
    /// Master switch (default: true). The updater is also inert for non-brew installs
    /// and dev builds, and can be turned off for a single run with `MMUX_NO_UPDATE`.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory, relative to the config dir (default: the config dir).
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessDef {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Start this process automatically when mmux first opens.
    #[serde(default)]
    pub autostart: bool,
}

/// How a desktop notification is delivered to the user's terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotifyMechanism {
    /// `OSC 9 ; message` — iTerm2, kitty, ghostty, WezTerm. The default: the widest
    /// support, at the cost of a single message field (no separate title — mmux folds
    /// the session name into the body).
    #[default]
    Osc9,
    /// `OSC 777 ; notify ; title ; body` — ghostty, foot, WezTerm, urxvt/VTE. Carries
    /// a real title, but is NOT understood by iTerm2 or kitty.
    Osc777,
    /// A bare terminal bell (BEL). Universal, but carries no message.
    Bell,
    /// Run an external command (see `command`). Useful for terminals with no
    /// notification escape — but, unlike the OSC mechanisms, it can't cross an SSH hop.
    Command,
}

/// Desktop-notification settings. Notifications fire when a session rings the bell
/// or emits a notification OSC of its own (e.g. an agent announcing it's done).
#[derive(Debug, Clone, Deserialize)]
pub struct NotifyConfig {
    /// Master switch (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Delivery mechanism (default: `osc9`).
    #[serde(default)]
    pub mechanism: NotifyMechanism,
    /// Only notify panes you're *not* currently looking at (default: true).
    #[serde(default = "default_true")]
    pub only_when_unfocused: bool,
    /// Minimum seconds between notifications from the same session (default: 5).
    #[serde(default = "default_throttle")]
    pub throttle_secs: u64,
    /// For `mechanism: command`: a shell command run with the notification exposed
    /// as `$MMUX_NOTIFY_TITLE` / `$MMUX_NOTIFY_BODY`. Falls back to a per-OS default
    /// (`osascript` on macOS, `notify-send` on Linux) when unset.
    #[serde(default)]
    pub command: Option<String>,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        NotifyConfig {
            enabled: true,
            mechanism: NotifyMechanism::default(),
            only_when_unfocused: true,
            throttle_secs: 5,
            command: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_throttle() -> u64 {
    5
}

impl Config {
    /// Load the effective config for `dir`: the global `~/.mmux/config.yaml` (if any)
    /// with the project's `mmux.yaml` (if any) merged on top.
    pub fn load(dir: &Path) -> Result<Config> {
        let global = load_file(global_config_path().as_deref())?;
        let project = load_file(config_path(dir).as_deref())?;

        let mut cfg = match (global, project) {
            (g, Some(p)) => merge(g, p),
            (Some(g), None) => g,
            (None, None) => {
                anyhow::bail!(
                    "no mmux.yaml in {} and no ~/.mmux/config.yaml. Run `mmux init` to create one.",
                    dir.display()
                )
            }
        };
        // All relative cwds (including those from the global config) resolve against
        // the project directory, so global agents/panels run in the current project.
        cfg.dir = dir.to_path_buf();
        Ok(cfg)
    }

    /// Load the root config for `dir`, then every directory it lists under
    /// `linked-projects`, into one [`Workspace`].
    ///
    /// Two rules make this safe even when a set of clones all share the same config
    /// (each listing the others, or itself):
    /// 1. **One level deep.** A linked project's own `linked-projects` is dropped —
    ///    aggregation is driven solely by the root, so links can't chain.
    /// 2. **De-dup by canonical path.** The root and every already-loaded project are
    ///    remembered; a path resolving to one of them is skipped. This drops self-
    ///    references and duplicates.
    /// A hard cap ([`MAX_PROJECTS`]) is the final backstop. Missing/unreadable links
    /// become warnings, never errors — only the root failing aborts.
    pub fn load_workspace(dir: &Path) -> Result<Workspace> {
        let root = Config::load(dir)?;
        let root_dir = root.dir.clone();
        let links = root.linked_projects.clone();

        let mut visited: HashSet<PathBuf> = HashSet::new();
        visited.insert(canonical(&root_dir));

        let mut projects = vec![root];
        let mut warnings = Vec::new();

        for raw in &links {
            if projects.len() >= MAX_PROJECTS {
                warnings.push(format!("linked-projects: capped at {MAX_PROJECTS}, ignoring the rest"));
                break;
            }
            let canon = canonical(&root_dir.join(raw));
            // Skip self, duplicates, and anything resolving to an already-loaded
            // project — this is what makes shared/clone configs safe.
            if !visited.insert(canon.clone()) {
                continue;
            }
            if !canon.is_dir() {
                warnings.push(format!("linked project not found: {raw}"));
                continue;
            }
            match Config::load(&canon) {
                Ok(mut c) => {
                    c.linked_projects.clear(); // one level only — never recurse
                    projects.push(c);
                }
                Err(e) => warnings.push(format!("skipped linked project {raw}: {e:#}")),
            }
        }

        Ok(Workspace { projects, warnings })
    }

    /// The workspace name to show: the configured `name`, or the directory's basename.
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| dir_basename(&self.dir))
    }

    /// Whether the git panel should be shown (subject to the dir being a repo).
    pub fn git_panel_enabled(&self) -> bool {
        self.git_panel.as_ref().map(|g| g.enabled).unwrap_or(true)
    }

    /// Whether background self-update is allowed by config (default: true). The updater
    /// applies further gates of its own (brew-managed, not a dev build); see [`crate::update`].
    pub fn auto_update_enabled(&self) -> bool {
        self.auto_update.as_ref().map(|a| a.enabled).unwrap_or(true)
    }
}

/// The directory's basename, or `"mmux"` if it has none (e.g. the filesystem root).
fn dir_basename(dir: &Path) -> String {
    dir.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mmux".into())
}

/// The project's own display name for `dir`, as the attach picker labels its rows:
/// the project `mmux.yaml`'s `name:` if set, else the directory's basename. Unlike
/// [`Config::display_name`] this reads only the *project* config, never the global
/// one — so a global `name:` can't leak onto every unrelated directory in the picker.
pub fn project_name(dir: &Path) -> String {
    load_file(config_path(dir).as_deref())
        .ok()
        .flatten()
        .and_then(|c| c.name)
        .unwrap_or_else(|| dir_basename(dir))
}

fn load_file(path: Option<&Path>) -> Result<Option<Config>> {
    let Some(path) = path else { return Ok(None) };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: Config =
        serde_yaml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(cfg))
}

/// Merge `project` on top of `base`: project values win. Agents and processes merge
/// by `name` (project entry replaces a same-named base entry, otherwise is appended).
fn merge(base: Option<Config>, project: Config) -> Config {
    let Some(base) = base else { return project };
    Config {
        name: project.name.or(base.name),
        agents: merge_named(base.agents, project.agents, |a| a.name.clone()),
        processes: merge_named(base.processes, project.processes, |p| p.name.clone()),
        git_panel: project.git_panel.or(base.git_panel),
        notifications: project.notifications.or(base.notifications),
        auto_update: project.auto_update.or(base.auto_update),
        // Linking is a per-project concern; the project file wins, falling back to
        // the global only if the project lists none.
        linked_projects: if project.linked_projects.is_empty() {
            base.linked_projects
        } else {
            project.linked_projects
        },
        dir: project.dir,
    }
}

/// Canonicalize `p`, falling back to the path as-given if it can't be resolved
/// (e.g. it doesn't exist), so de-dup still keys on something stable.
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn merge_named<T>(base: Vec<T>, over: Vec<T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let mut out = base;
    for item in over {
        let k = key(&item);
        match out.iter_mut().find(|x| key(x) == k) {
            Some(slot) => *slot = item,
            None => out.push(item),
        }
    }
    out
}

/// Returns the config path in `dir` if one exists.
pub fn config_path(dir: &Path) -> Option<PathBuf> {
    for name in ["mmux.yaml", "mmux.yml"] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// The file project-level edits (e.g. the in-TUI "+ New Process" form) write to:
/// the existing `mmux.yaml`/`.yml` if there is one, else a fresh `mmux.yaml`.
pub fn project_config_path(dir: &Path) -> PathBuf {
    config_path(dir).unwrap_or_else(|| dir.join("mmux.yaml"))
}

/// A process gathered by the in-TUI form, before it's written to the config.
pub struct ProcessDraft {
    pub name: String,
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub autostart: bool,
}

/// Split a typed command line ("npm run dev") into `(cmd, args)`, honouring simple
/// single/double quotes so one argument can contain spaces ("git commit -m 'a b'").
pub fn split_command(line: &str) -> (String, Vec<String>) {
    let mut parts = shell_split(line).into_iter();
    let cmd = parts.next().unwrap_or_default();
    (cmd, parts.collect())
}

/// Append `p` to the `processes:` list in `path`, preserving the file's existing
/// comments and layout — we edit the raw text rather than round-tripping through
/// serde (which would strip every comment). Creates the file/block if absent.
pub fn append_process(path: &Path, p: &ProcessDraft) -> Result<()> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let updated = insert_process(&original, p)?;
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Splice a rendered process item into `text`'s top-level `processes:` block (kept
/// pure for testing). The item lands among any existing entries — after the last
/// one, before trailing blank lines/comments — at their indentation. With no block
/// it's created at EOF; an `[]`/`null` placeholder is replaced by the real list.
fn insert_process(text: &str, p: &ProcessDraft) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(k) = lines.iter().position(|l| top_level_key(l) == Some("processes")) else {
        // No block yet: append a fresh one (with a blank separator) at EOF.
        let mut out = text.trim_end_matches('\n').to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("processes:\n");
        out.push_str(&render_item(p, 2));
        return Ok(out);
    };

    // An inline value other than an empty placeholder (`processes: foo`) is a shape
    // we can't safely extend by appending lines — leave it to the user.
    let after = lines[k].splitn(2, ':').nth(1).map(str::trim).unwrap_or("");
    let empty_marker = matches!(after, "" | "[]" | "{}" | "~" | "null");
    if !empty_marker {
        anyhow::bail!("`processes:` is written inline — add the entry by hand");
    }

    let indent = block_item_indent(&lines, k).unwrap_or(2);
    let item = render_item(p, indent);
    let at = block_end(&lines, k);
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i == at {
            out.push_str(&item);
        }
        // Drop an `[]`/`null` placeholder so the new block items parse as its value.
        if i == k && !after.is_empty() {
            out.push_str("processes:");
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    if at >= lines.len() {
        out.push_str(&item);
    }
    Ok(out)
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
fn shell_split(line: &str) -> Vec<String> {
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

/// Path to the global config (`~/.mmux/config.yaml`) if it exists.
pub fn global_config_path() -> Option<PathBuf> {
    global_config_target().filter(|p| p.exists())
}

/// Where the global config lives, whether or not it exists yet. The init wizard
/// uses this to create it; [`global_config_path`] is the existence-checked view.
pub fn global_config_target() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".mmux").join("config.yaml"))
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
# `name` is optional — it defaults to this directory's name.
# name: my-workspace

# Agents: interactive programs you spawn on demand. Each "+ New <name>" in the
# sidebar launches a fresh instance; its sidebar subtitle shows the terminal
# title the program sets, and a red dot appears when it rings the bell.
agents:
  - name: Claude
    cmd: claude
    args: ["--dangerously-skip-permissions"]
  - name: Codex
    cmd: codex
    args: ["--dangerously-bypass-approvals-and-sandbox"]

# Processes: defined commands you start/stop and watch. cwd is relative to this file.
processes:
  - name: Dev server
    cmd: npm
    args: ["run", "dev"]
    autostart: false

# Linked projects: sibling dirs (e.g. extra clones) to show alongside this one.
# Each gets its own group in the sidebar; switch with [ and ]. Listing is one level
# deep and de-duplicated by path, so you can drop this same config into every clone
# (even listing itself) without it ever expanding recursively.
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

    fn draft() -> ProcessDraft {
        ProcessDraft {
            name: "Dev server".into(),
            cmd: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            cwd: None,
            autostart: false,
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
        d.autostart = true;
        let out = insert_process("", &d).unwrap();
        assert!(!out.contains("args:"));
        assert!(out.contains("cwd: backend"));
        assert!(out.contains("autostart: true"));
    }

    #[test]
    fn yaml_scalar_quotes_only_when_needed() {
        assert_eq!(yaml_scalar("Dev server"), "Dev server");
        assert_eq!(yaml_scalar("."), ".");
        assert_eq!(yaml_scalar("../proj2"), "../proj2");
        assert_eq!(yaml_scalar("build:dev"), "\"build:dev\"");
        assert_eq!(yaml_scalar("- weird"), "\"- weird\"");
    }

    #[test]
    fn split_command_handles_quotes() {
        assert_eq!(split_command("npm run dev"), ("npm".into(), vec!["run".into(), "dev".into()]));
        assert_eq!(
            split_command("git commit -m 'a b'"),
            ("git".into(), vec!["commit".into(), "-m".into(), "a b".into()])
        );
        assert_eq!(split_command("  "), (String::new(), vec![]));
    }
}
