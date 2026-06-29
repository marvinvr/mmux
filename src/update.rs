//! Background self-update for Homebrew-installed builds.
//!
//! mmux's only install path today is the `marvinvr/homebrew-mmux` tap, so the updater
//! is brew-shaped: a cheap version *check* (curl the tap formula, compare its `version`
//! to ours) gates a heavier *install* (`brew update` + `brew upgrade mmux`). Both run on
//! throwaway threads and report back over an [`mpsc`](std::sync::mpsc) channel the TUI
//! drains in its tick loop — the same pattern the git panel uses for pull/push.
//!
//! Applying the update is a separate, user-gated step ([`exec_restart`]): brew can swap
//! the on-disk binary while mmux runs without disturbing anything, but running the *new*
//! code means replacing the inner process, which necessarily ends the live panes. So the
//! install happens automatically in the background; only the restart waits for the user.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// The version this binary was built as — what every check compares against.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Where CI publishes the formula every release. We read its `version` line directly
/// (rather than `brew outdated`, which only reflects the last `brew update`) so the
/// check is current, fast, and needs no global brew refresh.
const FORMULA_URL: &str =
    "https://raw.githubusercontent.com/marvinvr/homebrew-mmux/main/Formula/mmux.rb";

/// One finished background step, sent from a worker thread to the app's tick loop.
pub enum UpdateMsg {
    /// The check found a newer release; carries its version.
    Available(String),
    /// The check ran and we're already current.
    UpToDate,
    /// The background `brew upgrade` finished — the new binary is on disk.
    Installed(String),
    /// A step failed (network, brew, parse). Carries a short reason; surfaced quietly
    /// and retried on the next daily check.
    Failed(String),
}

/// The cheap, synchronous gate: should the updater run at all? Covers everything that
/// doesn't need a subprocess — config opt-out, a dev build, or the `MMUX_NO_UPDATE`
/// escape hatch. The brew-managed test is deferred into the worker thread so startup
/// never blocks on `brew`.
pub fn permitted(cfg_allows: bool) -> bool {
    cfg_allows
        && !cfg!(debug_assertions)
        && std::env::var_os("MMUX_NO_UPDATE").is_none()
}

/// Kick off a background check: verify we're a brew install, fetch the tap formula, and
/// report whether a newer version exists. Silent (sends nothing) for non-brew builds, so
/// a `cargo install`'d binary never shows a badge it can't act on.
///
/// First, cheaply and locally: a *sibling* mmux session may have already run the brew
/// upgrade while we keep running the old code. If the on-disk binary is newer than ours,
/// the update is effectively already staged — report it [`Installed`](UpdateMsg::Installed)
/// straight away and skip both the network check and a redundant `brew upgrade`.
pub fn spawn_check(tx: Sender<UpdateMsg>) {
    std::thread::spawn(move || {
        if !is_brew_managed() {
            return;
        }
        if let Some(v) = installed_newer() {
            let _ = tx.send(UpdateMsg::Installed(v));
            return;
        }
        let msg = match check_latest() {
            Ok(Some(v)) => UpdateMsg::Available(v),
            Ok(None) => UpdateMsg::UpToDate,
            Err(e) => UpdateMsg::Failed(e),
        };
        let _ = tx.send(msg);
    });
}

/// The version of the binary an in-place restart would land on (the brew symlink),
/// but only if it's strictly newer than the running one — i.e. a sibling session
/// already upgraded it. `None` if it's the same/older, or we can't read it.
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

/// Kick off the background install of `version` via brew. Reports [`UpdateMsg::Installed`]
/// once the new binary is linked, or [`UpdateMsg::Failed`] with a one-line reason.
pub fn spawn_install(tx: Sender<UpdateMsg>, version: String) {
    std::thread::spawn(move || {
        let msg = match run_install() {
            Ok(()) => UpdateMsg::Installed(version),
            Err(e) => UpdateMsg::Failed(e),
        };
        let _ = tx.send(msg);
    });
}

/// Replace this process image with a fresh `mmux --inner`, keeping the same tmux pane.
/// The caller restores the terminal first; on success this never returns. We invoke the
/// stable brew symlink rather than [`std::env::current_exe`], which after a `brew upgrade`
/// may point into a Cellar dir brew has already cleaned up. Returns the error on failure.
pub fn exec_restart() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    // `MMUX_INNER` / `MMUX_DIR` are already in our env (tmux set them) and are inherited
    // through exec, so the new image comes up as the inner TUI for the same directory —
    // which restores the previous agents/terminals from the saved state on startup, the
    // same as any other fresh open (see `crate::restore`).
    Command::new(resolve_exe()).arg("--inner").exec()
}

/// The binary to invoke for an in-place restart: the brew symlink if we can find it
/// (stable across upgrades), else the current exe, else a bare `mmux` for a PATH lookup.
fn resolve_exe() -> PathBuf {
    if let Some(prefix) = brew_prefix() {
        let p = prefix.join("bin").join("mmux");
        if p.exists() {
            return p;
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mmux"))
}

/// Whether this binary is managed by Homebrew — `brew` is on PATH and the running exe
/// lives under its prefix (`$(brew --prefix)/Cellar/…`). The one signal that the updater
/// can actually act: a `cargo install` or a dev build fails it and stays inert.
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

/// Fetch the tap formula and return `Some(version)` if it's newer than ours, else `None`.
fn check_latest() -> Result<Option<String>, String> {
    let rb = curl(FORMULA_URL)?;
    let latest =
        parse_formula_version(&rb).ok_or_else(|| "no version found in formula".to_string())?;
    Ok(version_gt(&latest, CURRENT).then_some(latest))
}

/// Refresh the tap so brew sees the new formula, then upgrade just mmux. Output is
/// captured (and discarded) so nothing leaks onto the TUI; `HOMEBREW_NO_AUTO_UPDATE`
/// keeps `upgrade` from refreshing a second time. Treats "already up to date" (a clean
/// exit) as success, so a sibling session that upgraded first doesn't read as a failure.
fn run_install() -> Result<(), String> {
    let update = Command::new("brew")
        .args(["update", "--quiet"])
        .output()
        .map_err(|e| format!("running brew: {e}"))?;
    if !update.status.success() {
        return Err(format!("brew update: {}", first_line(&update.stderr)));
    }
    let upgrade = Command::new("brew")
        .args(["upgrade", "mmux"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .output()
        .map_err(|e| format!("running brew: {e}"))?;
    if !upgrade.status.success() {
        return Err(format!("brew upgrade: {}", first_line(&upgrade.stderr)));
    }
    Ok(())
}

/// A single `curl -fsSL` GET with a short timeout, returning the body as text.
fn curl(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", url])
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl exited {}", out.status));
    }
    String::from_utf8(out.stdout).map_err(|_| "non-utf8 response".to_string())
}

/// Pull the `version "X.Y.Z"` value out of a Homebrew formula's text.
fn parse_formula_version(rb: &str) -> Option<String> {
    rb.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("version ")?;
        let v = rest.trim().trim_matches('"').trim();
        (!v.is_empty()).then(|| v.to_string())
    })
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

/// How often a long-running session re-checks for an update in the background.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

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
    fn parses_version_from_formula() {
        let rb = r#"
class Mmux < Formula
  desc "..."
  version "0.3.5"
  head "https://github.com/marvinvr/mmux.git", branch: "main"
"#;
        assert_eq!(parse_formula_version(rb).as_deref(), Some("0.3.5"));
        assert_eq!(parse_formula_version("no version here").as_deref(), None);
    }
}
