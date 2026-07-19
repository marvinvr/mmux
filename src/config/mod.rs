use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

mod yaml;
pub use yaml::{
    append_linked_project, append_process, global_agents, remove_process, replace_process,
    write_agents, write_starter,
};
pub(crate) use yaml::{render_agent_item, yaml_args, yaml_scalar};
pub(crate) use yaml::{
    GLOBAL_GIT_PANEL_HINT, GLOBAL_HEADER, PROJECT_AGENTS_COMMENT, PROJECT_AGENTS_EXAMPLE,
    PROJECT_HEADER, PROJECT_LINKED_COMMENT, PROJECT_LINKED_EXAMPLE, PROJECT_PROCESSES_COMMENT,
    PROJECT_PROCESSES_EXAMPLE,
};
use yaml::{quote_token, shell_split};

/// Upper bound on projects in one workspace (root + linked). A backstop so a
/// runaway `linked-projects` list can't explode the sidebar. Also the cap the
/// in-TUI "Link another project" browser enforces before growing the workspace.
pub(crate) const MAX_PROJECTS: usize = 8;

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
    /// Other project directories to load alongside this one in the same workspace —
    /// any related projects, not just extra clones (`../myproject2`). Each becomes its
    /// own group in the sidebar. Paths are relative to this config's dir. Honored only
    /// in the directory you launch mmux in: a linked project's own `linked-projects` is
    /// ignored, so a shared config can never expand recursively. See [`Config::load_workspace`].
    #[serde(default, rename = "linked-projects")]
    pub linked_projects: Vec<String>,
    /// Background self-update (Homebrew + native-binary installs). `None`/unset ⇒ enabled;
    /// see [`AutoUpdateConfig`] and [`crate::update`].
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

/// Settings for background self-update (acts on Homebrew and the mmux.org script's native
/// binary installs). When enabled, it checks for a newer release on startup and every 6
/// hours. A native-binary install downloads and stages it in the background, then shows a
/// quiet "restart to update" badge; a Homebrew install shows "update available" and applies
/// it with `brew upgrade mmux` once you confirm.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoUpdateConfig {
    /// Master switch (default: true). The updater is also inert for unmanaged installs
    /// (source builds, root-owned locations) and dev builds, and can be turned off for a
    /// single run with `MMUX_NO_UPDATE`.
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

/// A built-in agent harness mmux knows how to seed. Shared by the `mmux init` wizard
/// (which offers them as a multiselect) and the in-TUI agent manager (the sidebar's
/// `a` popup). Each optional flag set names a launch **posture** beyond the harness's
/// interactive default: `auto` auto-accepts file edits while still prompting for riskier
/// actions, `danger` skips approvals entirely ("danger mode"). Each is `None` when the
/// harness has no such mode. The manager cycles a row through the modes its preset
/// actually supports (see [`crate::agentmgr::Mode`]); a mode is a *sequence* of tokens
/// because some harnesses spell it as a flag + value (`--permission-mode acceptEdits`).
pub struct AgentPreset {
    pub name: &'static str,
    pub cmd: &'static str,
    /// Flags for the auto-accept-edits middle mode, if the harness has one.
    pub auto: Option<&'static [&'static str]>,
    /// Flags that skip every approval ("danger mode"); `None` for a harness with none.
    pub danger: Option<&'static [&'static str]>,
    pub blurb: &'static str,
}

/// The agent harnesses mmux offers out of the box. Every one ships a documented
/// danger-mode flag, and three also ship an auto-accept-edits middle mode; add new
/// harnesses here and they appear in both the wizard and the in-TUI agent manager
/// automatically. Flags verified against each tool's CLI.
pub const PRESETS: &[AgentPreset] = &[
    AgentPreset {
        name: "Claude",
        cmd: "claude",
        // The classifier-gated "auto mode" (newer than, and above, `acceptEdits`).
        auto: Some(&["--permission-mode", "auto"]),
        danger: Some(&["--dangerously-skip-permissions"]),
        blurb: "Anthropic Claude Code",
    },
    AgentPreset {
        name: "Codex",
        cmd: "codex",
        // Workspace-write sandbox: edits auto-apply in the workspace, shell stays
        // sandboxed. The middle tier between read-only and the full bypass below.
        auto: Some(&["--sandbox", "workspace-write"]),
        danger: Some(&["--dangerously-bypass-approvals-and-sandbox"]),
        blurb: "OpenAI Codex CLI",
    },
    AgentPreset {
        name: "Gemini",
        cmd: "gemini",
        auto: Some(&["--approval-mode", "auto_edit"]),
        danger: Some(&["--yolo"]),
        blurb: "Google Gemini CLI",
    },
    AgentPreset {
        name: "Amp",
        cmd: "amp",
        auto: None, // edits-vs-commands is allowlist/config-driven, not a CLI mode
        danger: Some(&["--dangerously-allow-all"]),
        blurb: "Sourcegraph Amp",
    },
    AgentPreset {
        name: "opencode",
        cmd: "opencode",
        auto: None, // granular approval lives in opencode.json, not a flag
        // opencode ships no `--yolo`; `--auto` is its "approve anything not denied" switch.
        danger: Some(&["--auto"]),
        blurb: "opencode terminal agent",
    },
    AgentPreset {
        name: "Grok",
        cmd: "grok",
        auto: None, // only a broad `--always-approve`; no edit-only tier
        danger: Some(&["--always-approve"]),
        blurb: "xAI Grok Build",
    },
];

/// The preset whose `name` matches `name` (case-sensitive), if any — lets the agent
/// manager map a configured agent back to its preset (for its blurb / danger flag).
pub fn preset_by_name(name: &str) -> Option<&'static AgentPreset> {
    PRESETS.iter().find(|p| p.name == name)
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
    /// Optional teardown command run in the process's directory after it stops — when
    /// you stop it (`x`) and when you quit mmux, but not on a restart. A shell line
    /// (run via `sh -c`), so `docker compose down` and the like work. `None`/unset ⇒
    /// nothing runs. Carried onto the live [`Session`](crate::app::Session) and executed
    /// there; see [`crate::app::Session::stop_command`].
    #[serde(default)]
    pub stop: Option<String>,
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
    /// with the project layer (`mmux.yaml` + an optional `mmux.local.yml`) merged on top.
    pub fn load(dir: &Path) -> Result<Config> {
        let global = load_file(global_config_path().as_deref())?;
        let project = load_project(dir)?;

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
    load_project(dir)
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

/// Load the project layer for `dir`: the project `mmux.yaml`/`.yml`, with an optional
/// `mmux.local.yml`/`.yaml` **deep-merged** on top. The local file is the developer's
/// private (usually git-ignored) override — and unlike the wholesale global→project
/// [`merge`], it is applied field-by-field (see [`deep_merge`]), so it can flip a
/// single nested key (say `notifications.enabled`) without restating its siblings.
/// It is honored only here, in the project directory — never for the global config.
/// Returns `None` when neither file exists.
fn load_project(dir: &Path) -> Result<Option<Config>> {
    let base = config_path(dir);
    let local = local_config_path(dir);
    if base.is_none() && local.is_none() {
        return Ok(None);
    }
    // Start from the project file's tree (or an empty mapping when only a local file
    // exists), then layer the local override on top, key by key.
    let mut value = match &base {
        Some(p) => load_value(p)?,
        None => serde_yaml::Value::Mapping(Default::default()),
    };
    if let Some(p) = &local {
        let over = load_value(p)?;
        // An empty/blank local file parses to `null` — a no-op override, not a request
        // to blank the whole config.
        if !over.is_null() {
            deep_merge(&mut value, over);
        }
    }
    let cfg: Config = serde_yaml::from_value(value)
        .with_context(|| format!("parsing project config in {}", dir.display()))?;
    Ok(Some(cfg))
}

/// Read and parse a YAML file into an untyped [`serde_yaml::Value`] (the form
/// [`deep_merge`] works on, before the merged tree is deserialized into [`Config`]).
fn load_value(path: &Path) -> Result<serde_yaml::Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Recursively layer `over` onto `base`, the rule behind the `mmux.local.yml` override:
/// - two **mappings** merge key by key (recursing into nested mappings), so an override
///   touches only the keys it names and leaves the siblings intact;
/// - two **named sequences** (every item a mapping with a scalar `name` — the shape of
///   `agents:`/`processes:`) merge by `name`: a same-named entry is deep-merged in place,
///   a new one is appended (matching how [`merge`] treats them);
/// - anything else (scalars, plain sequences like `args`/`linked-projects`, or a type
///   mismatch) is **replaced** wholesale by `over`.
fn deep_merge(base: &mut serde_yaml::Value, over: serde_yaml::Value) {
    use serde_yaml::Value;
    match (base, over) {
        (Value::Mapping(base), Value::Mapping(over)) => {
            for (k, v) in over {
                if base.contains_key(&k) {
                    deep_merge(base.get_mut(&k).unwrap(), v);
                } else {
                    base.insert(k, v);
                }
            }
        }
        (Value::Sequence(base), Value::Sequence(over))
            if is_named_seq(base) && is_named_seq(&over) =>
        {
            merge_named_seq(base, over);
        }
        (base, over) => *base = over,
    }
}

/// Whether `seq` is a non-empty sequence of mappings that each carry a scalar `name` —
/// i.e. an `agents:`/`processes:` list, which [`deep_merge`] merges by name. An empty
/// list is not "named", so an empty override sequence clears the base instead.
fn is_named_seq(seq: &[serde_yaml::Value]) -> bool {
    !seq.is_empty() && seq.iter().all(|v| v.get("name").and_then(|n| n.as_str()).is_some())
}

/// Merge `over`'s items into `base` by their `name`: a same-named entry is deep-merged
/// in place (so a partial override needn't restate the whole entry), a new one appended.
fn merge_named_seq(base: &mut Vec<serde_yaml::Value>, over: Vec<serde_yaml::Value>) {
    use serde_yaml::Value;
    for item in over {
        let name = item.get("name").and_then(Value::as_str).map(str::to_owned);
        let pos = base
            .iter()
            .position(|x| x.get("name").and_then(Value::as_str) == name.as_deref());
        match pos {
            Some(i) => deep_merge(&mut base[i], item),
            None => base.push(item),
        }
    }
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
pub(crate) fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// A path from directory `from` to directory `to`, in the form written into
/// `linked-projects` (e.g. `../sibling`). Both should be absolute (ideally
/// canonical). Walks up to the common ancestor with `..`, then down into `to`;
/// returns `.` when they're the same directory and `to` verbatim when they share no
/// prefix (different roots). Pure — unit-tested.
pub(crate) fn relative_path(from: &Path, to: &Path) -> String {
    use std::path::Component;
    let f: Vec<Component> = from.components().collect();
    let t: Vec<Component> = to.components().collect();
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return to.to_string_lossy().into_owned();
    }
    let mut parts: Vec<String> = std::iter::repeat("..".to_string()).take(f.len() - common).collect();
    for c in &t[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
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

/// Returns the local-override path in `dir` if one exists (`mmux.local.yaml`/`.yml`).
/// This is the private, per-developer file deep-merged over the project config; see
/// [`load_project`]. Project-only (there is no global counterpart by design).
pub fn local_config_path(dir: &Path) -> Option<PathBuf> {
    for name in ["mmux.local.yaml", "mmux.local.yml"] {
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
    /// Optional teardown shell line (see [`ProcessDef::stop`]); emitted only when set.
    pub stop: Option<String>,
}

/// An agent gathered by the in-TUI agent manager, before it's written to the config.
/// Unlike the process form there's no wizard of fields — the manager toggles presets
/// on/off and cycles their launch mode, so a draft is just the three rendered keys.
pub struct AgentDraft {
    pub name: String,
    pub cmd: String,
    pub args: Vec<String>,
}

/// Split a typed command line ("npm run dev") into `(cmd, args)`, honouring simple
/// single/double quotes so one argument can contain spaces ("git commit -m 'a b'").
pub fn split_command(line: &str) -> (String, Vec<String>) {
    let mut parts = shell_split(line).into_iter();
    let cmd = parts.next().unwrap_or_default();
    (cmd, parts.collect())
}

/// Join a stored `cmd` + `args` back into a single editable command line — the
/// inverse of [`split_command`], used to pre-fill the process-edit form. Tokens
/// containing whitespace (or empty ones) are double-quoted so a re-split round-trips.
pub fn join_command(cmd: &str, args: &[String]) -> String {
    let mut out = quote_token(cmd);
    for a in args {
        out.push(' ');
        out.push_str(&quote_token(a));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_command_round_trips_through_split() {
        assert_eq!(join_command("npm", &["run".into(), "dev".into()]), "npm run dev");
        // A token with spaces is quoted so a re-split keeps it as one argument.
        let joined = join_command("git", &["commit".into(), "-m".into(), "a b".into()]);
        assert_eq!(joined, "git commit -m \"a b\"");
        assert_eq!(split_command(&joined), ("git".into(), vec!["commit".into(), "-m".into(), "a b".into()]));
    }

    #[test]
    fn presets_are_well_formed() {
        let claude = preset_by_name("Claude").unwrap();
        assert_eq!(claude.cmd, "claude");
        assert_eq!(claude.danger, Some(&["--dangerously-skip-permissions"][..]));
        assert_eq!(claude.auto, Some(&["--permission-mode", "auto"][..]));
        assert!(preset_by_name("Nope").is_none());
        // Every shipped preset has a command and a danger flag.
        assert!(PRESETS.iter().all(|p| !p.cmd.is_empty() && p.danger.is_some()));
    }

    #[test]
    fn relative_path_walks_to_the_common_ancestor() {
        assert_eq!(relative_path(Path::new("/u/m/proj"), Path::new("/u/m/other")), "../other");
        assert_eq!(relative_path(Path::new("/u/m/proj"), Path::new("/u/m/proj")), ".");
        assert_eq!(relative_path(Path::new("/u/m/proj"), Path::new("/u/m/proj/sub")), "sub");
        assert_eq!(relative_path(Path::new("/u/m/a/b"), Path::new("/u/m/x")), "../../x");
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

    // ── merge: project layered over global ───────────────────────────────────
    fn cfg(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).expect("test config parses")
    }

    #[test]
    fn merge_returns_project_verbatim_when_no_base() {
        let project = cfg("name: proj\nagents:\n  - name: Claude\n    cmd: claude\n");
        let merged = merge(None, project);
        assert_eq!(merged.name.as_deref(), Some("proj"));
        assert_eq!(merged.agents.len(), 1);
    }

    #[test]
    fn merge_replaces_scalar_blocks_wholesale_else_falls_back() {
        let base = cfg("name: global\ngit-panel:\n  enabled: true\nnotifications:\n  enabled: false\n");
        let project = cfg("name: proj\ngit-panel:\n  enabled: false\n");
        let merged = merge(Some(base), project);
        // Project's name and git-panel win outright…
        assert_eq!(merged.name.as_deref(), Some("proj"));
        assert!(!merged.git_panel.unwrap().enabled);
        // …but a block the project doesn't set falls back to the global wholesale
        // (no field-level merge — the global's notifications come through intact).
        assert!(!merged.notifications.unwrap().enabled);
    }

    #[test]
    fn merge_name_falls_back_to_base_when_project_unset() {
        let base = cfg("name: global\n");
        let project = cfg("agents: []\n"); // no name set
        assert_eq!(merge(Some(base), project).name.as_deref(), Some("global"));
    }

    #[test]
    fn merge_agents_and_processes_by_name() {
        let base = cfg("agents:\n  - name: Claude\n    cmd: claude\n  - name: Codex\n    cmd: codex\n");
        let project = cfg("agents:\n  - name: Claude\n    cmd: claude-beta\n  - name: Gemini\n    cmd: gemini\n");
        let merged = merge(Some(base), project);
        let names: Vec<&str> = merged.agents.iter().map(|a| a.name.as_str()).collect();
        // Same-name entry is replaced in place; a new one is appended; the untouched survives.
        assert_eq!(names, ["Claude", "Codex", "Gemini"]);
        assert_eq!(merged.agents[0].cmd, "claude-beta");
    }

    #[test]
    fn merge_linked_projects_project_wins_else_base() {
        let base = || cfg("linked-projects:\n  - ../g1\n  - ../g2\n");
        // Project lists none → inherit the global list.
        let inherited = merge(Some(base()), cfg("name: p\n"));
        assert_eq!(inherited.linked_projects, vec!["../g1", "../g2"]);
        // Project lists some → its list replaces the global's outright (no concat).
        let overridden = merge(Some(base()), cfg("linked-projects:\n  - ../p1\n"));
        assert_eq!(overridden.linked_projects, vec!["../p1"]);
    }

    #[test]
    fn merge_dir_comes_from_the_project() {
        let base = cfg("name: g\n");
        let mut project = cfg("name: p\n");
        project.dir = PathBuf::from("/work/proj");
        assert_eq!(merge(Some(base), project).dir, PathBuf::from("/work/proj"));
    }

    // ── deep_merge: mmux.local.yml layered over the project config ────────────
    fn deep(base: &str, over: &str) -> serde_yaml::Value {
        let mut b: serde_yaml::Value = serde_yaml::from_str(base).unwrap();
        let o: serde_yaml::Value = serde_yaml::from_str(over).unwrap();
        deep_merge(&mut b, o);
        b
    }

    #[test]
    fn deep_merge_overrides_one_nested_key_keeping_siblings() {
        // The whole point: flip notifications.enabled without restating mechanism.
        let merged = deep(
            "notifications:\n  enabled: true\n  mechanism: bell\n  throttle_secs: 9\n",
            "notifications:\n  enabled: false\n",
        );
        let cfg: Config = serde_yaml::from_value(merged).unwrap();
        let n = cfg.notifications.unwrap();
        assert!(!n.enabled); // overridden
        assert_eq!(n.mechanism, NotifyMechanism::Bell); // sibling preserved
        assert_eq!(n.throttle_secs, 9); // sibling preserved
    }

    #[test]
    fn deep_merge_named_seq_merges_by_name_and_appends() {
        // A partial agent override touches one field; same-named entry deep-merges,
        // a brand-new agent is appended, untouched ones survive.
        let merged = deep(
            "agents:\n  - name: Claude\n    cmd: claude\n    args: [\"--old\"]\n  - name: Codex\n    cmd: codex\n",
            "agents:\n  - name: Claude\n    args: [\"--new\"]\n  - name: Gemini\n    cmd: gemini\n",
        );
        let cfg: Config = serde_yaml::from_value(merged).unwrap();
        let names: Vec<&str> = cfg.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Claude", "Codex", "Gemini"]);
        // Claude keeps its cmd (not restated) but takes the new args.
        assert_eq!(cfg.agents[0].cmd, "claude");
        assert_eq!(cfg.agents[0].args, vec!["--new"]);
    }

    #[test]
    fn deep_merge_replaces_plain_sequences_wholesale() {
        // `args` is a plain (unnamed) sequence, so the override replaces it outright.
        let merged = deep(
            "agents:\n  - name: Claude\n    cmd: claude\n    args: [\"a\", \"b\"]\n",
            "agents:\n  - name: Claude\n    args: [\"c\"]\n",
        );
        let cfg: Config = serde_yaml::from_value(merged).unwrap();
        assert_eq!(cfg.agents[0].args, vec!["c"]);
    }

    #[test]
    fn deep_merge_empty_named_seq_clears_the_base() {
        // `processes: []` in the local file means "no processes here", not a no-op.
        let merged = deep("processes:\n  - name: Dev\n    cmd: npm\n", "processes: []\n");
        let cfg: Config = serde_yaml::from_value(merged).unwrap();
        assert!(cfg.processes.is_empty());
    }
}
