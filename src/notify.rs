//! Desktop notifications, delivered as terminal escape sequences.
//!
//! mmux runs inside its per-directory tmux jail, so the way it raises a
//! notification on the *user's* machine — even across an SSH hop — is to write a
//! notification escape to its controlling terminal and let that terminal emulator
//! render the native popup. The bytes ride the normal output stream, so the local
//! and remote cases are the same code path. Inside tmux the escape is wrapped in a
//! DCS passthrough so the jail forwards it instead of swallowing it (paired with
//! `allow-passthrough on`, set in `tmux.rs`).

use crate::config::NotifyMechanism;
use std::process::{Command, Stdio};

/// A resolved notification: title + body, already prefixed with the session name
/// by the caller.
pub struct Note {
    pub title: String,
    pub body: String,
}

impl Note {
    /// The bytes to write to the controlling terminal for `mech`, or `None` for
    /// mechanisms that aren't terminal escapes (`Command`). Inside tmux the escape
    /// is wrapped in passthrough so it reaches the outer terminal.
    pub fn escape_bytes(&self, mech: NotifyMechanism, in_tmux: bool) -> Option<Vec<u8>> {
        let esc = self.escape(mech)?;
        let wrapped = if in_tmux { tmux_passthrough(&esc) } else { esc };
        Some(wrapped.into_bytes())
    }

    /// The raw, unwrapped escape sequence for `mech`.
    fn escape(&self, mech: NotifyMechanism) -> Option<String> {
        match mech {
            NotifyMechanism::Osc9 => {
                // OSC 9 carries a single message, so fold title and body together.
                let msg = if self.body.is_empty() {
                    self.title.clone()
                } else {
                    format!("{} — {}", self.title, self.body)
                };
                Some(format!("\x1b]9;{}\x07", sanitize(&msg)))
            }
            NotifyMechanism::Osc777 => Some(format!(
                "\x1b]777;notify;{};{}\x07",
                sanitize(&self.title),
                sanitize(&self.body)
            )),
            NotifyMechanism::Bell => Some("\x07".to_string()),
            NotifyMechanism::Command => None,
        }
    }
}

/// Fire an external-command notification (the `command` mechanism). Runs `cmd`
/// (or a per-OS default) via the shell with the notification exposed as env vars,
/// detached and silent. Local only — it can't cross an SSH hop.
pub fn run_command(cmd: Option<&str>, note: &Note) {
    let Some(template) = cmd.map(str::to_string).or_else(default_command) else {
        return;
    };
    let _ = Command::new("sh")
        .arg("-c")
        .arg(template)
        .env("MMUX_NOTIFY_TITLE", &note.title)
        .env("MMUX_NOTIFY_BODY", &note.body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// A reasonable OS-default command, reading the env vars `run_command` sets.
fn default_command() -> Option<String> {
    if cfg!(target_os = "macos") {
        Some(
            r#"osascript -e "display notification \"$MMUX_NOTIFY_BODY\" with title \"$MMUX_NOTIFY_TITLE\"""#
                .to_string(),
        )
    } else if cfg!(target_os = "linux") {
        Some(r#"notify-send "$MMUX_NOTIFY_TITLE" "$MMUX_NOTIFY_BODY""#.to_string())
    } else {
        None
    }
}

/// Wrap a terminal escape so tmux forwards it to the outer terminal:
/// `ESC P tmux ; <payload, every ESC doubled> ESC \`. Needs `allow-passthrough on`.
fn tmux_passthrough(payload: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", payload.replace('\x1b', "\x1b\x1b"))
}

/// Drop control characters that would prematurely terminate or corrupt the OSC string.
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            title: "Claude".into(),
            body: "done".into(),
        }
    }

    #[test]
    fn osc777_outside_tmux() {
        let b = note().escape_bytes(NotifyMechanism::Osc777, false).unwrap();
        assert_eq!(b, b"\x1b]777;notify;Claude;done\x07");
    }

    #[test]
    fn osc9_folds_title_and_body() {
        let b = note().escape_bytes(NotifyMechanism::Osc9, false).unwrap();
        assert_eq!(b, "\x1b]9;Claude — done\x07".as_bytes());
    }

    #[test]
    fn tmux_wraps_and_doubles_escapes() {
        let s = String::from_utf8(note().escape_bytes(NotifyMechanism::Osc777, true).unwrap())
            .unwrap();
        assert!(s.starts_with("\x1bPtmux;"));
        assert!(s.ends_with("\x1b\\"));
        // The inner OSC's introducing ESC is doubled so tmux passes it through.
        assert!(s.contains("\x1b\x1b]777;notify;Claude;done\x07"));
    }

    #[test]
    fn bell_is_just_bel() {
        let b = note().escape_bytes(NotifyMechanism::Bell, false).unwrap();
        assert_eq!(b, b"\x07");
    }

    #[test]
    fn command_has_no_escape() {
        assert!(note()
            .escape_bytes(NotifyMechanism::Command, false)
            .is_none());
    }

    #[test]
    fn sanitize_strips_control_bytes() {
        let n = Note {
            title: "a\x07b".into(),
            body: "c\nd".into(),
        };
        let b = n.escape_bytes(NotifyMechanism::Osc777, false).unwrap();
        assert_eq!(b, b"\x1b]777;notify;ab;cd\x07");
    }
}
