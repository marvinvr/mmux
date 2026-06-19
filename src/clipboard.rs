//! Putting text on the system clipboard from inside the TUI.
//!
//! Two best-effort, complementary paths:
//! - **OSC 52**: an escape sequence written to our own stdout. Works over SSH and
//!   through the tmux jail (which we configure with `set-clipboard on`), as long as
//!   the outer terminal honours it (iTerm2 with the setting on, kitty, wezterm, …).
//! - **A CLI helper** (`pbcopy`/`wl-copy`/`xclip`/`xsel`) when one is on PATH, which
//!   covers local terminals that ignore OSC 52 (e.g. stock Terminal.app).

use std::io::Write;
use std::process::{Command, Stdio};

/// Copy `text` to the clipboard via OSC 52 and any available CLI helper.
pub fn copy(text: &str) {
    osc52(text);
    via_helper(text);
}

/// Write the OSC 52 set-clipboard sequence to our stdout. The event loop flushes
/// frames separately, so we flush here to push the sequence out immediately.
fn osc52(text: &str) {
    let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

/// Pipe `text` into the first available platform clipboard helper.
fn via_helper(text: &str) {
    // (command, args). First one that spawns wins.
    let helpers: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["-i", "-b"]),
    ];
    for (cmd, args) in helpers {
        let spawned = Command::new(cmd)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = spawned {
            // Drop the stdin handle (closing the pipe) before waiting, or the
            // helper would block reading forever.
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
}

/// Standard base64 for the OSC 52 payload. Kept tiny to avoid pulling in a crate.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((chunk[0] as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}
