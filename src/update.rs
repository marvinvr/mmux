//! Background self-update for release builds.
//!
//! mmux ships two managed install paths, and the updater serves both from one flow:
//!
//! - **Homebrew** (macOS): the binary lives under `$(brew --prefix)`. A newer release is
//!   *offered*, not auto-applied — the About card asks first, then runs `brew upgrade mmux`
//!   for the user (brew's own bookkeeping must drive its own upgrade, so we can't just swap
//!   the file underneath it).
//! - **Self-managed** (the `mmux.org/install.sh` binary, typically in `~/.local/bin`): a
//!   release binary we can atomically replace ourselves. Here the update is silent —
//!   downloaded and staged in the background, exactly like the old brew flow.
//!
//! Both kinds share one version check: follow the GitHub `releases/latest` redirect and
//! read the tag (a plain web redirect — no REST API, no rate limit, no token, no hosting).
//! Everything runs on throwaway threads reporting over an [`mpsc`](std::sync::mpsc) channel
//! the TUI drains in its tick loop — the same pattern the git panel uses for pull/push.
//!
//! Applying is always a separate, user-gated step ([`exec_restart`]): staging the new
//! binary is safe while mmux runs, but running the *new* code means replacing the inner
//! process, which necessarily ends the live panes. So only the restart waits for the user.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// The version this binary was built as — what every check compares against.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The GitHub "latest release" URL. A GET redirects (302) to `…/releases/tag/vX.Y.Z`;
/// we read the tag off the final URL. Serves both install kinds as the single version
/// source (the tap formula and the release tag are bumped from the same CI run).
const RELEASES_LATEST: &str = "https://github.com/marvinvr/mmux/releases/latest";

/// Where the per-target tarballs are attached on each release.
const DOWNLOAD_BASE: &str = "https://github.com/marvinvr/mmux/releases/download";

/// Whether this binary was built by the release pipeline (CI sets `MMUX_RELEASE=1`). Only
/// release binaries self-manage: a `cargo install` from source lacks the stamp and stays
/// inert, matching the old "non-brew installs don't self-update" behaviour. Homebrew is
/// detected separately (it may build its Linux fallback from source without the stamp).
const IS_RELEASE_BUILD: bool = option_env!("MMUX_RELEASE").is_some();

/// How mmux was installed — decides how (and whether) an update is applied.
#[derive(Clone, Copy)]
pub enum InstallKind {
    /// Under Homebrew's prefix: upgrade via `brew upgrade mmux`, after the user confirms.
    Brew,
    /// A release binary in a user-writable location: download the new tarball and swap it
    /// in place ourselves, no confirmation needed.
    SelfManaged,
}

/// One finished background step, sent from a worker thread to the app's tick loop.
pub enum UpdateMsg {
    /// The check found a newer release. `kind` decides the app's next move: a self-managed
    /// install downloads it straight away; a brew install waits for the user to confirm.
    Available { version: String, kind: InstallKind },
    /// The check ran and we're already current.
    UpToDate,
    /// The new binary is on disk and staged — a restart applies it.
    Installed(String),
    /// A step failed (network, brew, tar, parse). Carries a short reason; surfaced quietly
    /// and retried on the next periodic check.
    Failed(String),
    /// This isn't a managed install, so the updater can't act. A terminal verdict (no
    /// retry buys anything): it resolves the optimistic `Checking` state the UI starts in
    /// into a quiet "self-update off" rather than leaving the About card spinning forever.
    NotManaged,
}

/// The cheap, synchronous gate: should the updater run at all? Covers everything that
/// doesn't need a subprocess — config opt-out, a dev build, or the `MMUX_NO_UPDATE`
/// escape hatch. The install-kind test is deferred into the worker thread so startup
/// never blocks on `brew` or a filesystem probe.
pub fn permitted(cfg_allows: bool) -> bool {
    cfg_allows
        && !cfg!(debug_assertions)
        && std::env::var_os("MMUX_NO_UPDATE").is_none()
}

/// Kick off a background check: classify the install, then report whether a newer version
/// exists (and, for a found update, how it should be applied). An unmanaged build reports
/// [`NotManaged`](UpdateMsg::NotManaged) and stops — enough for the UI to resolve its
/// optimistic `Checking` state into a quiet "off". (It can't decline silently: the caller
/// already flipped to `Checking` before spawning us, so a worker that returns without
/// sending leaves the About card spinning forever.)
///
/// First, cheaply and locally: a *sibling* session may have already upgraded the on-disk
/// binary (brew relinked it, or another mmux self-installed) while we keep running the old
/// code. If what a restart would launch is newer than ours, the update is effectively
/// staged — report it [`Installed`](UpdateMsg::Installed) and skip the network check.
pub fn spawn_check(tx: Sender<UpdateMsg>) {
    std::thread::spawn(move || {
        let Some(kind) = install_kind() else {
            let _ = tx.send(UpdateMsg::NotManaged);
            return;
        };
        if let Some(v) = installed_newer() {
            let _ = tx.send(UpdateMsg::Installed(v));
            return;
        }
        let msg = match check_latest() {
            Ok(Some(v)) => UpdateMsg::Available { version: v, kind },
            Ok(None) => UpdateMsg::UpToDate,
            Err(e) => UpdateMsg::Failed(e),
        };
        let _ = tx.send(msg);
    });
}

/// The version of the binary an in-place restart would land on, but only if it's strictly
/// newer than the running one — i.e. a sibling session already upgraded it. `None` if it's
/// the same/older, or we can't read it.
fn installed_newer() -> Option<String> {
    let out = Command::new(resolve_exe()).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    // `mmux --version` prints `mmux X.Y.Z`.
    let text = String::from_utf8(out.stdout).ok()?;
    let v = text.split_whitespace().nth(1)?.trim().to_string();
    version_gt(&v, CURRENT).then_some(v)
}

/// Kick off the background install of `version`. For a self-managed install this downloads
/// and swaps the binary; for brew it runs `brew upgrade mmux` (already confirmed by the
/// user). Reports [`UpdateMsg::Installed`] once the new binary is on disk, or
/// [`UpdateMsg::Failed`] with a one-line reason.
pub fn spawn_install(tx: Sender<UpdateMsg>, version: String, kind: InstallKind) {
    std::thread::spawn(move || {
        let result = match kind {
            InstallKind::Brew => run_brew_upgrade(),
            InstallKind::SelfManaged => run_self_install(&version),
        };
        let _ = tx.send(match result {
            Ok(()) => UpdateMsg::Installed(version),
            Err(e) => UpdateMsg::Failed(e),
        });
    });
}

/// Replace this process image with a fresh `mmux --inner`, keeping the same tmux pane.
/// The caller restores the terminal first; on success this never returns. Returns the
/// error on failure.
pub fn exec_restart() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    // `MMUX_INNER` / `MMUX_DIR` are already in our env (tmux set them) and are inherited
    // through exec, so the new image comes up as the inner TUI for the same directory —
    // which restores the previous agents/terminals from the saved state on startup, the
    // same as any other fresh open (see `crate::restore`).
    Command::new(resolve_exe()).arg("--inner").exec()
}

/// The binary to invoke for an in-place restart. For a brew install we prefer the brew
/// symlink (stable across upgrades, where [`std::env::current_exe`] may point into a
/// Cellar dir brew has already cleaned up). For a self-managed install brew's `bin/mmux`
/// doesn't exist, so this naturally falls through to `current_exe` — the path we just
/// swapped the new binary onto.
fn resolve_exe() -> PathBuf {
    if let Some(prefix) = brew_prefix() {
        let p = prefix.join("bin").join("mmux");
        if p.exists() {
            return p;
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mmux"))
}

/// Classify this install, or `None` if the updater can't act on it. Brew wins first (it may
/// build its Linux fallback from source, so the release stamp isn't required there); a
/// stamped release binary in a user-writable dir on a target we ship for is self-managed;
/// everything else (source builds, root-owned installs, unshipped targets) stays inert.
fn install_kind() -> Option<InstallKind> {
    if is_brew_managed() {
        return Some(InstallKind::Brew);
    }
    if IS_RELEASE_BUILD && asset_target().is_some() && exe_dir_writable() {
        return Some(InstallKind::SelfManaged);
    }
    None
}

/// Whether this binary is managed by Homebrew — `brew` is on PATH and the running exe
/// lives under its prefix (`$(brew --prefix)/Cellar/…`).
fn is_brew_managed() -> bool {
    let Some(prefix) = brew_prefix() else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    // Resolve the bin/ symlink into the Cellar so the prefix test holds on Linux too
    // (macOS already hands back a canonical path).
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    exe.starts_with(&prefix)
}

/// Can we atomically replace the running binary in place? The self-managed path stages the
/// new binary beside the old one and renames over it, which needs write access to the
/// *directory*. Probed by creating (and removing) a temp file there — the honest test,
/// since permission bits alone miss ownership.
fn exe_dir_writable() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let Some(dir) = exe.parent() else {
        return false;
    };
    let probe = dir.join(format!(".mmux-writable-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// `$(brew --prefix)` (e.g. `/opt/homebrew` or `/usr/local`), or `None` if brew is absent.
fn brew_prefix() -> Option<PathBuf> {
    let out = Command::new("brew").arg("--prefix").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// The release-asset target triple for this OS/arch, or `None` for a platform we don't
/// ship a prebuilt binary for. Linux ships static musl binaries (run on any distro
/// regardless of glibc); macOS ships an Apple-silicon binary only (Intel Macs are EOL and
/// install from source, so they never self-manage).
fn asset_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        _ => None,
    }
}

/// Fetch the latest release version and return `Some(version)` if it's newer than ours.
fn check_latest() -> Result<Option<String>, String> {
    let latest = latest_version()?;
    Ok(version_gt(&latest, CURRENT).then_some(latest))
}

/// Follow the `releases/latest` redirect and read the version off the resulting
/// `…/releases/tag/vX.Y.Z` URL. A plain web redirect, so no REST API rate limit, no token.
fn latest_version() -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", "-o", "/dev/null", "-w", "%{url_effective}"])
        .arg(RELEASES_LATEST)
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl exited {}", out.status));
    }
    let url = String::from_utf8(out.stdout).map_err(|_| "non-utf8 response".to_string())?;
    parse_tag_version(&url).ok_or_else(|| "no release tag in redirect".to_string())
}

/// Pull `X.Y.Z` out of a `…/releases/tag/vX.Y.Z` URL (leading `v` stripped). Returns `None`
/// if the tail isn't a version — e.g. a repo with no releases redirects to `…/releases`.
fn parse_tag_version(url: &str) -> Option<String> {
    let tag = url.trim().trim_end_matches('/').rsplit('/').next()?;
    let v = tag.trim_start_matches('v').trim();
    (v.chars().next().is_some_and(|c| c.is_ascii_digit())).then(|| v.to_string())
}

/// Run the confirmed `brew upgrade mmux`. Output is captured (and discarded) so nothing
/// leaks onto the TUI. We let brew do its implicit auto-update — that's what lets `upgrade`
/// see the freshly-pushed formula without a separate `brew update` step. Since it's
/// user-initiated (not a background loop), the extra moment brew spends refreshing is fine.
fn run_brew_upgrade() -> Result<(), String> {
    let out = Command::new("brew")
        .args(["upgrade", "mmux"])
        .output()
        .map_err(|e| format!("running brew: {e}"))?;
    if !out.status.success() {
        return Err(format!("brew upgrade: {}", first_line(&out.stderr)));
    }
    Ok(())
}

/// Download the release tarball for `version`, extract it, and atomically swap it over the
/// running binary. Everything is staged in a temp dir *beside* the live binary so the final
/// rename is atomic (same filesystem); replacing the path the process launched from is safe
/// on Unix (the kernel keeps the old inode alive until we exec or exit). Integrity rests on
/// the HTTPS download from github.com (`curl -f` fails on any non-2xx), so there's no
/// separate checksum step — the transport already authenticates the bytes.
fn run_self_install(version: &str) -> Result<(), String> {
    let target = asset_target().ok_or_else(|| "unsupported platform".to_string())?;
    let exe = std::env::current_exe().map_err(|e| format!("locating binary: {e}"))?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let dir = exe.parent().ok_or_else(|| "binary has no parent directory".to_string())?;

    let url = format!("{DOWNLOAD_BASE}/v{version}/mmux-{target}.tar.gz");
    let stage = dir.join(format!(".mmux-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir(&stage).map_err(|e| format!("staging dir: {e}"))?;

    // Run the fallible middle in a closure so the staging dir is always cleaned up after.
    let result = install_from(&url, &stage, &exe);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// The fallible core of [`run_self_install`]: download → extract → (re-sign) → swap.
fn install_from(url: &str, stage: &Path, exe: &Path) -> Result<(), String> {
    let tarball = stage.join("mmux.tar.gz");
    curl_to_file(url, &tarball)?;

    // `tar` is present on macOS and Linux; extract into the staging dir (never over the
    // live binary), yielding `mmux` there.
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(stage)
        .status()
        .map_err(|e| format!("running tar: {e}"))?;
    if !status.success() {
        return Err("tar failed to extract the release".to_string());
    }
    let fresh = stage.join("mmux");
    if !fresh.exists() {
        return Err("release archive did not contain mmux".to_string());
    }
    set_executable(&fresh)?;

    // A relocated ad-hoc-signed macOS binary gets SIGKILL'd ("Killed: 9") on first run
    // unless re-signed in place — the same fix the Homebrew formula applies.
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&fresh)
            .status();
    }

    std::fs::rename(&fresh, exe).map_err(|e| format!("replacing binary: {e}"))
}

/// `chmod 755` — tar usually preserves the bit, but make sure the swapped-in binary runs.
fn set_executable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(p)
        .map_err(|e| format!("stat: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(p, perms).map_err(|e| format!("chmod: {e}"))
}

/// A `curl -fsSL` download to `path` with a generous timeout for the binary tarball.
fn curl_to_file(url: &str, path: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "120", "-o"])
        .arg(path)
        .arg(url)
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {}", first_line(&out.stderr)));
    }
    Ok(())
}

/// True if `a` is strictly newer than `b`, comparing dot-separated numeric components
/// left to right (so `0.10.0` > `0.9.9`). A leading `v` is ignored; non-numeric or
/// missing components count as 0 rather than erroring.
fn version_gt(a: &str, b: &str) -> bool {
    fn parts(s: &str) -> Vec<u64> {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The first non-empty line of captured stderr, for a compact one-line error.
fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("unknown error")
        .to_string()
}

/// How often a long-running session re-checks for an update in the background. Timed
/// from each session's startup (not the wall clock), so independent sessions stagger
/// their checks rather than all hitting GitHub at once.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_gt_compares_numerically() {
        assert!(version_gt("0.10.0", "0.9.9")); // not lexicographic
        assert!(version_gt("1.0.0", "0.9.9"));
        assert!(version_gt("0.3.5", "0.3.4"));
        assert!(!version_gt("0.3.4", "0.3.4")); // equal is not newer
        assert!(!version_gt("0.3.3", "0.3.4"));
        assert!(version_gt("v0.3.5", "0.3.4")); // a leading v is ignored
        assert!(version_gt("0.4", "0.3.9")); // ragged tails: missing parts are 0
        assert!(!version_gt("0.3", "0.3.0"));
    }

    #[test]
    fn parses_version_from_release_redirect() {
        assert_eq!(
            parse_tag_version("https://github.com/marvinvr/mmux/releases/tag/v0.9.0").as_deref(),
            Some("0.9.0")
        );
        assert_eq!(
            parse_tag_version("https://github.com/marvinvr/mmux/releases/tag/v1.2.3/").as_deref(),
            Some("1.2.3")
        );
        // A repo with no releases redirects to the bare list — not a version.
        assert_eq!(
            parse_tag_version("https://github.com/marvinvr/mmux/releases"),
            None
        );
    }

    #[test]
    fn asset_target_covers_shipped_platforms() {
        // Whatever host runs the tests must map to one of the three shipped triples
        // (or None on an unshipped platform, e.g. Intel macOS — still valid, non-panicking).
        let t = asset_target();
        if let Some(t) = t {
            assert!(t.contains("apple-darwin") || t.contains("linux-musl"));
        }
    }
}
