use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

mod yaml;
pub use yaml::{
    append_process, global_agents, remove_process, replace_process, write_agents, write_starter,
    write_workspace,
};
use yaml::{quote_token, shell_split};
pub(crate) use yaml::{render_agent_item, yaml_args, yaml_scalar};
pub(crate) use yaml::{
    GLOBAL_GIT_PANEL_HINT, GLOBAL_HEADER, PROJECT_AGENTS_COMMENT, PROJECT_AGENTS_EXAMPLE,
    PROJECT_HEADER, PROJECT_PROCESSES_COMMENT, PROJECT_PROCESSES_EXAMPLE,
    PROJECT_WORKSPACE_COMMENT, PROJECT_WORKSPACE_EXAMPLE,
};

/// Upper bound on the projects one workspace manifest loads. A backstop so a
/// runaway `folders:` list can't explode the sidebar.
pub(crate) const MAX_PROJECTS: usize = 10;

/// A workspace config, loaded from `mmux.yaml` (or `mmux.yml`) in a directory.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Optional display name for this directory/session (terminal title, plus the
    /// sidebar title for an ordinary single-project session).
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
    /// Marks this directory as a **workspace manifest**: a named bundle of project
    /// folders that open together in one sidebar, each folder its own group. The
    /// manifest's directory is a container, not a project — list `.` under
    /// [`WorkspaceDef::folders`] to include it too. A project-file concern only (a
    /// global `workspace:` is ignored — see [`Config::load`]).
    /// See [`Config::load_workspace`].
    #[serde(default)]
    pub workspace: Option<WorkspaceDef>,
    /// Background self-update (Homebrew + native-binary installs). `None`/unset ⇒ enabled;
    /// see [`AutoUpdateConfig`] and [`crate::update`].
    #[serde(default, rename = "auto-update")]
    pub auto_update: Option<AutoUpdateConfig>,
    /// The directory the config was loaded from. Relative `cwd`s resolve against this.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// The `workspace:` block of a manifest config. Presence of the block (even an empty
/// one) is what makes a config a manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceDef {
    /// Member project folders, relative to the manifest's directory, in sidebar order.
    /// Each becomes its own project group; `.` includes the manifest's own directory.
    #[serde(default)]
    pub folders: Vec<String>,
}

/// A loaded workspace: either a single project (the dir mmux was launched in), or —
/// when that dir's config carries a [`workspace:`](WorkspaceDef) manifest — the
/// manifest's folders, each loaded as its own project. Always non-empty.
pub struct Workspace {
    /// The launch directory (the manifest's dir for a manifest workspace). This — not
    /// `projects[0].dir`, which for a manifest is the first *member* — keys the tmux
    /// session and the restore-state file, so a workspace and a member folder opened
    /// solo never share state.
    pub dir: PathBuf,
    /// The effective config of the directory mmux was launched in. For a manifest
    /// this owns workspace-level identity and settings (`name`, notifications,
    /// auto-update) even though the directory is not itself a project unless `.` is
    /// listed under `folders`.
    pub config: Config,
    /// Whether this workspace came from a `workspace:` manifest. Drives
    /// workspace-specific presentation and reload behavior.
    pub manifest: bool,
    pub projects: Vec<Config>,
    /// Non-fatal problems (a folder that was missing, unreadable, or beyond the cap)
    /// to surface without aborting startup.
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
            // A workspace manifest is a per-directory fact; a global `workspace:`
            // would turn every unconfigured directory into that workspace.
            (Some(mut g), None) => {
                g.workspace = None;
                g
            }
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

    /// Load the effective [`Workspace`] for `dir`: its config alone when it's a plain
    /// project, or — when the config carries a [`workspace:`](WorkspaceDef) manifest —
    /// each of the manifest's `folders` as its own project, in list order.
    ///
    /// Manifest expansion is deliberately flat and bounded:
    /// - **One level deep.** A member whose own config is also a manifest loads as a
    ///   plain project (its `workspace:` block dropped, with a warning) — workspaces
    ///   never nest.
    /// - **De-dup by canonical path.** A folder resolving to an already-loaded one is
    ///   skipped, so duplicates (and a `.` next to an absolute spelling of the same
    ///   dir) collapse.
    /// - A hard cap ([`MAX_PROJECTS`]) is the final backstop. Missing/unreadable
    ///   folders become warnings, never errors — only the manifest itself failing
    ///   aborts, and a manifest whose folders *all* fail falls back to opening its
    ///   own directory as a plain project.
    pub fn load_workspace(dir: &Path) -> Result<Workspace> {
        let root = Config::load(dir)?;
        let root_dir = root.dir.clone();
        let mut warnings = Vec::new();

        // The `linked-projects` key was replaced by workspace manifests; serde now
        // ignores it silently, so surface a pointer instead of quietly un-bundling.
        if has_removed_linked_projects(dir) {
            warnings.push(
                "linked-projects was removed — bundle projects with a `workspace:` manifest instead (see `mmux docs`)".into(),
            );
        }

        let Some(ws) = root.workspace.clone() else {
            return Ok(Workspace {
                dir: root_dir,
                config: root.clone(),
                manifest: false,
                projects: vec![root],
                warnings,
            });
        };

        let root_canon = canonical(&root_dir);
        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut projects: Vec<Config> = Vec::new();
        for raw in &ws.folders {
            if projects.len() >= MAX_PROJECTS {
                warnings.push(format!(
                    "workspace: capped at {MAX_PROJECTS} folders, ignoring the rest"
                ));
                break;
            }
            let canon = canonical(&root_dir.join(raw));
            if !visited.insert(canon.clone()) {
                continue;
            }
            if !canon.is_dir() {
                warnings.push(format!("workspace folder not found: {raw}"));
                continue;
            }
            match Config::load(&canon) {
                Ok(mut c) => {
                    if canon != root_canon && has_removed_linked_projects(&canon) {
                        warnings.push(format!(
                            "{raw}: linked-projects was removed — this member's old links are ignored"
                        ));
                    }
                    // Workspaces never nest: a member that is itself a manifest loads
                    // as a plain project. `.` (the manifest's own dir) is the expected
                    // self-reference, so only foreign manifests get the warning.
                    if c.workspace.take().is_some() && canon != root_canon {
                        warnings.push(format!(
                            "{raw}: nested workspace ignored — loaded as a plain project"
                        ));
                    }
                    projects.push(c);
                }
                Err(e) => warnings.push(format!("skipped workspace folder {raw}: {e:#}")),
            }
        }

        if projects.is_empty() {
            warnings.push(
                "workspace lists no loadable folders — opening this directory as a plain project"
                    .into(),
            );
            let mut root = root;
            root.workspace = None;
            return Ok(Workspace {
                dir: root_dir,
                config: root.clone(),
                manifest: false,
                projects: vec![root],
                warnings,
            });
        }

        Ok(Workspace {
            dir: root_dir,
            config: root,
            manifest: true,
            projects,
            warnings,
        })
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

/// The directory identity used by the attach picker: its project-layer `name:` (or
/// folder basename) plus whether it declares a `workspace:` manifest. This reads only
/// the project config, never the global one — so a global name or workspace cannot
/// leak onto every unrelated directory in the picker.
pub(crate) fn project_identity(dir: &Path) -> (String, bool) {
    let cfg = load_project(dir).ok().flatten();
    let workspace = cfg.as_ref().and_then(|c| c.workspace.as_ref()).is_some();
    let name = cfg
        .and_then(|c| c.name)
        .unwrap_or_else(|| dir_basename(dir));
    (name, workspace)
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
/// - anything else (scalars, plain sequences like `args`/`folders`, or a type
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
    !seq.is_empty()
        && seq
            .iter()
            .all(|v| v.get("name").and_then(|n| n.as_str()).is_some())
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
        // A manifest is a per-directory fact: only the project file can declare one
        // (a global `workspace:` must not turn every directory into that workspace).
        workspace: project.workspace,
        dir: project.dir,
    }
}

/// Canonicalize `p`, falling back to the path as-given if it can't be resolved
/// (e.g. it doesn't exist), so de-dup still keys on something stable.
pub(crate) fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Whether either project-layer file still declares the removed top-level
/// `linked-projects:` key. Serde intentionally ignores unknown fields, so this
/// targeted raw check preserves a useful migration warning.
fn has_removed_linked_projects(dir: &Path) -> bool {
    [config_path(dir), local_config_path(dir)]
        .into_iter()
        .flatten()
        .any(|path| {
            std::fs::read_to_string(path)
                .map(|text| {
                    text.lines()
                        .any(|line| line.starts_with("linked-projects:"))
                })
                .unwrap_or(false)
        })
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

/// The layer a workspace editor should update. A private local override that already
/// owns `workspace:` remains the owner; otherwise edit the ordinary project file.
/// This avoids a successful-looking save being shadowed immediately by
/// `mmux.local.y{a,}ml` during the next load.
pub fn workspace_config_path(dir: &Path) -> PathBuf {
    if let Some(local) = local_config_path(dir) {
        let owns_workspace = std::fs::read_to_string(&local)
            .ok()
            .and_then(|text| serde_yaml::from_str::<serde_yaml::Value>(&text).ok())
            .and_then(|value| value.as_mapping().cloned())
            .is_some_and(|map| map.contains_key(&serde_yaml::Value::String("workspace".into())));
        if owns_workspace {
            return local;
        }
    }
    project_config_path(dir)
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
        assert_eq!(
            join_command("npm", &["run".into(), "dev".into()]),
            "npm run dev"
        );
        // A token with spaces is quoted so a re-split keeps it as one argument.
        let joined = join_command("git", &["commit".into(), "-m".into(), "a b".into()]);
        assert_eq!(joined, "git commit -m \"a b\"");
        assert_eq!(
            split_command(&joined),
            (
                "git".into(),
                vec!["commit".into(), "-m".into(), "a b".into()]
            )
        );
    }

    #[test]
    fn presets_are_well_formed() {
        let claude = preset_by_name("Claude").unwrap();
        assert_eq!(claude.cmd, "claude");
        assert_eq!(claude.danger, Some(&["--dangerously-skip-permissions"][..]));
        assert_eq!(claude.auto, Some(&["--permission-mode", "auto"][..]));
        assert!(preset_by_name("Nope").is_none());
        // Every shipped preset has a command and a danger flag.
        assert!(PRESETS
            .iter()
            .all(|p| !p.cmd.is_empty() && p.danger.is_some()));
    }

    #[test]
    fn split_command_handles_quotes() {
        assert_eq!(
            split_command("npm run dev"),
            ("npm".into(), vec!["run".into(), "dev".into()])
        );
        assert_eq!(
            split_command("git commit -m 'a b'"),
            (
                "git".into(),
                vec!["commit".into(), "-m".into(), "a b".into()]
            )
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
        let base =
            cfg("name: global\ngit-panel:\n  enabled: true\nnotifications:\n  enabled: false\n");
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
        let base =
            cfg("agents:\n  - name: Claude\n    cmd: claude\n  - name: Codex\n    cmd: codex\n");
        let project = cfg(
            "agents:\n  - name: Claude\n    cmd: claude-beta\n  - name: Gemini\n    cmd: gemini\n",
        );
        let merged = merge(Some(base), project);
        let names: Vec<&str> = merged.agents.iter().map(|a| a.name.as_str()).collect();
        // Same-name entry is replaced in place; a new one is appended; the untouched survives.
        assert_eq!(names, ["Claude", "Codex", "Gemini"]);
        assert_eq!(merged.agents[0].cmd, "claude-beta");
    }

    #[test]
    fn merge_workspace_comes_from_the_project_only() {
        let base = || cfg("workspace:\n  folders:\n    - ../g1\n");
        // A global `workspace:` never leaks onto a project…
        let plain = merge(Some(base()), cfg("name: p\n"));
        assert!(plain.workspace.is_none());
        // …while a project manifest survives the merge untouched.
        let manifest = merge(Some(base()), cfg("workspace:\n  folders:\n    - ../p1\n"));
        let ws = manifest.workspace.expect("project manifest kept");
        assert_eq!(ws.folders, vec!["../p1"]);
    }

    #[test]
    fn workspace_block_parses_with_defaults() {
        // A bare `workspace:` block (or one with only folders) is a valid manifest.
        let c = cfg("workspace:\n  folders:\n    - one\n    - two\n");
        let ws = c.workspace.expect("manifest");
        assert_eq!(ws.folders, vec!["one", "two"]);
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
        let merged = deep(
            "processes:\n  - name: Dev\n    cmd: npm\n",
            "processes: []\n",
        );
        let cfg: Config = serde_yaml::from_value(merged).unwrap();
        assert!(cfg.processes.is_empty());
    }

    // ── load_workspace: manifest expansion ────────────────────────────────────

    /// A throwaway directory tree for manifest tests, removed on drop.
    struct TempTree(PathBuf);
    impl TempTree {
        fn new(tag: &str) -> TempTree {
            let dir = std::env::temp_dir().join(format!("mmux-ws-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempTree(dir)
        }
        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }
    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_workspace_expands_manifest_folders() {
        let t = TempTree::new("expand");
        t.write(
            "hub/mmux.yaml",
            "name: Hub\nworkspace:\n  folders:\n    - ../a\n    - ../a\n    - ../b\n    - ../missing\n    - .\n",
        );
        t.write("a/mmux.yaml", "name: A\n");
        // A member that is itself a manifest must load as a plain project.
        t.write(
            "b/mmux.yaml",
            "name: B\nworkspace:\n  folders:\n    - ../a\n",
        );
        let ws = Config::load_workspace(&t.0.join("hub")).unwrap();
        assert!(ws.manifest);
        assert_eq!(canonical(&ws.dir), canonical(&t.0.join("hub")));
        // `../a` once (deduped), B flattened, then `.` — the hub itself, last.
        let names: Vec<String> = ws.projects.iter().map(Config::display_name).collect();
        assert_eq!(names, ["A", "B", "Hub"]);
        assert!(ws.projects.iter().all(|c| c.workspace.is_none()));
        assert!(ws.warnings.iter().any(|w| w.contains("../missing")));
        assert!(ws.warnings.iter().any(|w| w.contains("nested workspace")));
    }

    #[test]
    fn load_workspace_without_manifest_is_single_project() {
        let t = TempTree::new("plain");
        t.write("p/mmux.yaml", "name: Solo\n");
        let ws = Config::load_workspace(&t.0.join("p")).unwrap();
        assert!(!ws.manifest);
        assert_eq!(ws.projects.len(), 1);
        assert_eq!(ws.projects[0].display_name(), "Solo");
    }

    #[test]
    fn load_workspace_caps_folders_and_warns() {
        let t = TempTree::new("cap");
        let list: String = (0..12).map(|i| format!("    - ../p{i}\n")).collect();
        t.write("hub/mmux.yaml", &format!("workspace:\n  folders:\n{list}"));
        for i in 0..12 {
            t.write(&format!("p{i}/mmux.yaml"), "processes: []\n");
        }
        let ws = Config::load_workspace(&t.0.join("hub")).unwrap();
        assert_eq!(ws.projects.len(), MAX_PROJECTS);
        assert!(ws.warnings.iter().any(|w| w.contains("capped")));
    }

    #[test]
    fn load_workspace_warns_on_lingering_linked_projects() {
        let t = TempTree::new("deprecated");
        t.write("p/mmux.yaml", "name: Old\nlinked-projects:\n  - ../x\n");
        let ws = Config::load_workspace(&t.0.join("p")).unwrap();
        // The removed key no longer bundles anything…
        assert!(!ws.manifest);
        assert_eq!(ws.projects.len(), 1);
        // …but the load points at its replacement rather than staying silent.
        assert!(ws.warnings.iter().any(|w| w.contains("linked-projects")));
    }

    #[test]
    fn load_workspace_empty_manifest_falls_back_to_plain() {
        let t = TempTree::new("empty");
        t.write("hub/mmux.yaml", "name: Hub\nworkspace:\n  folders: []\n");
        let ws = Config::load_workspace(&t.0.join("hub")).unwrap();
        assert!(!ws.manifest);
        assert_eq!(ws.projects.len(), 1);
        assert!(ws.projects[0].workspace.is_none());
        assert!(ws
            .warnings
            .iter()
            .any(|w| w.contains("no loadable folders")));
    }
}
