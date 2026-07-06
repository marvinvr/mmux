//! Open a URL in the user's default browser, best-effort.
//!
//! mmux draws through the tmux jail, but the opener talks to the OS directly
//! (launch services on macOS, the desktop portal on Linux), so tmux doesn't get
//! in the way. Fire-and-forget: we spawn on a throwaway thread and reap it there
//! so the UI thread never blocks and no zombie is left behind. A missing opener
//! (`open`/`xdg-open` not on PATH) silently does nothing.

use std::process::{Command, Stdio};
use std::thread;

/// Open `url` in the default browser.
pub fn url(url: &str) {
    let url = url.to_string();
    thread::spawn(move || {
        if let Ok(mut child) = spawn(&url) {
            let _ = child.wait();
        }
    });
}

/// Spawn the platform URL opener, output silenced.
fn spawn(url: &str) -> std::io::Result<std::process::Child> {
    let (cmd, pre) = opener();
    Command::new(cmd)
        .args(pre)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// The default URL opener for the platform: `(command, leading args)`.
fn opener() -> (&'static str, &'static [&'static str]) {
    #[cfg(target_os = "macos")]
    return ("open", &[]);
    #[cfg(target_os = "windows")]
    return ("cmd", &["/C", "start", ""]);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return ("xdg-open", &[]);
}
