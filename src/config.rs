use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

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
/// `a` popup). `danger` is the single flag that opts the agent out of permission/
/// approval prompts ("danger mode"); `None` for a harness with no such flag.
pub struct AgentPreset {
    pub name: &'static str,
    pub cmd: &'static str,
    pub danger: Option<&'static str>,
    pub blurb: &'static str,
}

/// The agent harnesses mmux offers out of the box. Every one ships a documented
/// danger-mode flag; add new harnesses here and they appear in both the wizard and
/// the in-TUI agent manager automatically. Flags verified against each tool's CLI.
pub const PRESETS: &[AgentPreset] = &[
    AgentPreset {
        name: "Claude",
        cmd: "claude",
        danger: Some("--dangerously-skip-permissions"),
        blurb: "Anthropic Claude Code",
    },
    AgentPreset {
        name: "Codex",
        cmd: "codex",
        danger: Some("--dangerously-bypass-approvals-and-sandbox"),
        blurb: "OpenAI Codex CLI",
    },
    AgentPreset {
        name: "Gemini",
        cmd: "gemini",
        danger: Some("--yolo"),
        blurb: "Google Gemini CLI",
    },
    AgentPreset {
        name: "Amp",
        cmd: "amp",
        danger: Some("--dangerously-allow-all"),
        blurb: "Sourcegraph Amp",
    },
    AgentPreset {
        name: "opencode",
        cmd: "opencode",
        danger: Some("--yolo"),
        blurb: "opencode terminal agent",
    },
    AgentPreset {
        name: "Grok",
        cmd: "grok",
        danger: Some("--always-approve"),
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
/// on/off and flips their danger flag, so a draft is just the three rendered keys.
pub struct AgentDraft {
    pub name: String,
    pub cmd: String,
    pub args: Vec<String>,
}

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

/// Double-quote a command-line token when a bare word wouldn't survive [`shell_split`]
/// (it holds whitespace, or is empty). No escaping — enough to round-trip typed input.
fn quote_token(s: &str) -> String {
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
fn render_agent_item(a: &AgentDraft, indent: usize) -> String {
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
        let mut out = text.trim_end_matches('\n').to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&block);
        return out;
    };
    let end = block_end(&lines, k);
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i == k {
            out.push_str(&block); // ends with a newline
        }
        if i >= k && i < end {
            continue; // drop the old `agents:` line and its items
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// A fresh, self-documenting global config (`~/.mmux/config.yaml`) seeded with
/// `agents_block`. Matches the header the `mmux init` wizard writes for a first-run
/// global file, so a config born from the in-TUI agent manager explains itself.
fn scaffold_global_file(agents_block: &str) -> String {
    let mut s = String::new();
    s.push_str("# mmux global config (~/.mmux/config.yaml).\n");
    s.push_str("# Agents here are available in EVERY project. A project's mmux.yaml can\n");
    s.push_str("# override or add to them by name.\n");
    s.push_str("# Full guide: run `mmux docs`, or visit https://mmux.org.\n\n");
    s.push_str(agents_block);
    s.push_str("\n# A git panel is shown automatically in every git repo. To disable it:\n");
    s.push_str("# git-panel:\n");
    s.push_str("#   enabled: false\n");
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
    s.push_str("# mmux workspace config.\n");
    s.push_str("# Run `mmux` in this directory to open (or reattach to) the session.\n");
    s.push_str("# New here? Run `mmux docs` for the full guide, or visit https://mmux.org.\n");
    s.push_str("# `name` is optional — it defaults to this directory's name.\n");
    s.push_str("# name: my-workspace\n\n");

    s.push_str("# Agents: interactive programs you spawn on demand from the sidebar.\n");
    s.push_str("# agents:\n");
    s.push_str("#   - name: Claude\n#     cmd: claude\n#     args: [\"--dangerously-skip-permissions\"]\n\n");

    s.push_str("# Processes: commands you start/stop and watch. cwd is relative to this file.\n");
    s.push_str("# An optional `stop:` shell line (e.g. docker compose down) runs in that dir when\n");
    s.push_str("# the process is stopped or mmux quits — handy for tearing down what it started.\n");
    if processes.is_empty() {
        s.push_str("# processes:\n");
        s.push_str("#   - name: Dev server\n#     cmd: npm\n#     args: [\"run\", \"dev\"]\n#     autostart: false\n#     # stop: docker compose down\n\n");
    } else {
        s.push_str("processes:\n");
        s.push_str(processes);
        s.push('\n');
    }

    s.push_str("# Linked projects: other projects to show alongside this one in the same\n");
    s.push_str("# workspace — any directories you want grouped together (extra clones, a\n");
    s.push_str("# related repo, a service), each its own sidebar group. One level deep,\n");
    s.push_str("# de-duplicated by path.\n");
    if linked.is_empty() {
        s.push_str("# linked-projects:\n");
        s.push_str("#   - ../myproject2\n");
    } else {
        s.push_str("linked-projects:\n");
        s.push_str(linked);
    }
    s
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
        let mut out = text.trim_end_matches('\n').to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(block);
        out.push_str(":\n");
        out.push_str(&render(2));
        return Ok(out);
    };

    // An inline value other than an empty placeholder (`block: foo`) is a shape we
    // can't safely extend by appending lines — leave it to the user.
    let after = lines[k].splitn(2, ':').nth(1).map(str::trim).unwrap_or("");
    let empty_marker = matches!(after, "" | "[]" | "{}" | "~" | "null");
    if !empty_marker {
        anyhow::bail!("`{block}:` is written inline — add the entry by hand");
    }

    let indent = block_item_indent(&lines, k).unwrap_or(2);
    let item = render(indent);
    let at = block_end(&lines, k);
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i == at {
            out.push_str(&item);
        }
        // Drop an `[]`/`null` placeholder so the new block items parse as its value.
        if i == k && !after.is_empty() {
            out.push_str(block);
            out.push(':');
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
    let rendered = render_item(p, indent);
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i == start {
            out.push_str(&rendered);
        }
        if i >= start && i < end {
            continue; // drop the old item's lines
        }
        out.push_str(l);
        out.push('\n');
    }
    Ok(out)
}

/// Delete the `block:` list item named `name` from `text`, preserving the file's other
/// comments and layout (kept pure for testing). Errors if it can't be located.
fn delete_named_item(text: &str, block: &str, name: &str) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end, _) = named_item_span(&lines, block, name)
        .ok_or_else(|| anyhow::anyhow!("couldn't find “{name}” under `{block}:` — edit it by hand"))?;
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i >= start && i < end {
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    Ok(out)
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
    fn join_command_round_trips_through_split() {
        assert_eq!(join_command("npm", &["run".into(), "dev".into()]), "npm run dev");
        // A token with spaces is quoted so a re-split keeps it as one argument.
        let joined = join_command("git", &["commit".into(), "-m".into(), "a b".into()]);
        assert_eq!(joined, "git commit -m \"a b\"");
        assert_eq!(split_command(&joined), ("git".into(), vec!["commit".into(), "-m".into(), "a b".into()]));
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
    fn presets_are_well_formed() {
        let claude = preset_by_name("Claude").unwrap();
        assert_eq!(claude.cmd, "claude");
        assert_eq!(claude.danger, Some("--dangerously-skip-permissions"));
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
