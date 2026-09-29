//! Pure translation from crossterm key events to the byte sequences a PTY
//! expects. No app state — easy to read and to unit-test.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode a crossterm key event into the bytes to send to the PTY. `kitty_flags`
/// is the progressive-keyboard mode requested by the program in that pane.
/// Returns an empty vec for keys mmux doesn't forward.
pub fn encode_key(k: &KeyEvent, kitty_flags: u8) -> Vec<u8> {
    use KeyCode::*;
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);

    // With Kitty's disambiguation flag active, modified Enter must retain its
    // modifier: prompt TUIs use Shift+Enter/Alt+Enter to insert a newline while
    // plain Enter submits. Outside that negotiated mode, keep terminal-compatible
    // legacy behavior (all Enter variants are CR).
    if k.code == Enter {
        let modifier = kitty_modifier(k.modifiers);
        if kitty_flags & 1 != 0 && modifier != 1 {
            return format!("\x1b[13;{modifier}u").into_bytes();
        }
        return vec![b'\r'];
    }

    let mut out: Vec<u8> = match k.code {
        Char(c) => {
            if ctrl {
                let b = (c as u8).to_ascii_uppercase();
                if (0x40..0x80).contains(&b) {
                    vec![b & 0x1f]
                } else {
                    let mut buf = [0u8; 4];
                    c.encode_utf8(&mut buf).as_bytes().to_vec()
                }
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        Backspace => vec![0x7f],
        Tab => vec![b'\t'],
        BackTab => vec![27, 91, 90],
        Esc => vec![27],
        Left => vec![27, 91, 68],
        Right => vec![27, 91, 67],
        Up => vec![27, 91, 65],
        Down => vec![27, 91, 66],
        Home => vec![27, 91, 72],
        End => vec![27, 91, 70],
        PageUp => vec![27, 91, 53, 126],
        PageDown => vec![27, 91, 54, 126],
        Delete => vec![27, 91, 51, 126],
        Insert => vec![27, 91, 50, 126],
        F(n) => match n {
            1 => vec![27, 79, 80],
            2 => vec![27, 79, 81],
            3 => vec![27, 79, 82],
            4 => vec![27, 79, 83],
            5 => vec![27, 91, 49, 53, 126],
            6 => vec![27, 91, 49, 55, 126],
            7 => vec![27, 91, 49, 56, 126],
            8 => vec![27, 91, 49, 57, 126],
            9 => vec![27, 91, 50, 48, 126],
            10 => vec![27, 91, 50, 49, 126],
            11 => vec![27, 91, 50, 51, 126],
            12 => vec![27, 91, 50, 52, 126],
            _ => vec![],
        },
        _ => vec![],
    };

    // Alt prefixes the sequence with ESC.
    if alt && !out.is_empty() {
        let mut v = vec![27];
        v.append(&mut out);
        return v;
    }
    out
}

/// Parse a tmux-style key name — `Enter`, `Escape`, `Tab`, `BSpace`, `Up`, `PageDown`,
/// `F5`, `Space`, a single character, optionally prefixed by `C-` (Ctrl), `M-` (Alt)
/// and `S-` (Shift): `C-c`, `M-x`, `S-Enter` — into the key event [`encode_key`] turns
/// into bytes. `None` for anything else, which the control socket types as literal text.
pub fn parse_key_name(name: &str) -> Option<KeyEvent> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = name;
    // A modifier prefix only counts when something follows it: a bare `C-` is text.
    while rest.len() > 2 {
        let modifier = match rest.get(..2) {
            Some("C-" | "c-") => KeyModifiers::CONTROL,
            Some("M-" | "m-" | "A-" | "a-") => KeyModifiers::ALT,
            Some("S-" | "s-") => KeyModifiers::SHIFT,
            _ => break,
        };
        mods |= modifier;
        rest = &rest[2..];
    }
    let mut chars = rest.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyEvent::new(KeyCode::Char(c), mods));
    }
    let code = match rest.to_ascii_lowercase().as_str() {
        "enter" | "return" | "cr" => KeyCode::Enter,
        "escape" | "esc" => KeyCode::Esc,
        "tab" if mods.contains(KeyModifiers::SHIFT) => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "btab" | "backtab" => KeyCode::BackTab,
        "bspace" | "backspace" | "bs" => KeyCode::Backspace,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" | "ppage" => KeyCode::PageUp,
        "pagedown" | "pgdn" | "npage" => KeyCode::PageDown,
        "delete" | "del" | "dc" => KeyCode::Delete,
        "insert" | "ins" | "ic" => KeyCode::Insert,
        f if f.len() > 1 && f.starts_with('f') => match f[1..].parse::<u8>() {
            Ok(n @ 1..=12) => KeyCode::F(n),
            _ => return None,
        },
        _ => return None,
    };
    Some(KeyEvent::new(code, mods))
}

/// Kitty numbers modifiers as one plus a bitset: Shift, Alt, Ctrl, Super,
/// Hyper, Meta. Keep this local to the protocol-specific path above.
fn kitty_modifier(modifiers: KeyModifiers) -> u8 {
    1 + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL))
        + 8 * u8::from(modifiers.contains(KeyModifiers::SUPER))
        + 16 * u8::from(modifiers.contains(KeyModifiers::HYPER))
        + 32 * u8::from(modifiers.contains(KeyModifiers::META))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_char() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), 0),
            b"a"
        );
    }

    #[test]
    fn ctrl_c_is_0x03() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), 0),
            vec![0x03]
        );
    }

    #[test]
    fn ctrl_b_is_0x02() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('b'), KeyModifiers::CONTROL), 0),
            vec![0x02]
        );
    }

    #[test]
    fn enter_is_carriage_return() {
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::NONE), 1),
            b"\r"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), 0),
            b"\r"
        );
    }

    #[test]
    fn modified_enter_uses_negotiated_kitty_encoding() {
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::SHIFT), 1),
            b"\x1b[13;2u"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::ALT), 1),
            b"\x1b[13;3u"
        );
        assert_eq!(
            encode_key(
                &key(KeyCode::Enter, KeyModifiers::SHIFT | KeyModifiers::CONTROL),
                1
            ),
            b"\x1b[13;6u"
        );
    }

    #[test]
    fn backspace_is_del() {
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE), 0),
            vec![0x7f]
        );
    }

    #[test]
    fn arrows_are_csi() {
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::NONE), 0),
            vec![27, 91, 65]
        );
        assert_eq!(
            encode_key(&key(KeyCode::Down, KeyModifiers::NONE), 0),
            vec![27, 91, 66]
        );
    }

    #[test]
    fn alt_prefixes_esc() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('x'), KeyModifiers::ALT), 0),
            vec![27, b'x']
        );
    }

    #[test]
    fn parses_tmux_style_key_names() {
        let enc = |n: &str| encode_key(&parse_key_name(n).unwrap(), 0);
        assert_eq!(enc("Enter"), b"\r");
        assert_eq!(enc("C-c"), vec![0x03]);
        assert_eq!(enc("Escape"), vec![27]);
        assert_eq!(enc("Up"), vec![27, 91, 65]);
        assert_eq!(enc("M-x"), vec![27, b'x']);
        assert_eq!(enc("S-Tab"), vec![27, 91, 90]);
        assert_eq!(enc("Space"), b" ");
        assert_eq!(enc("y"), b"y");
        assert_eq!(enc("F5"), vec![27, 91, 49, 53, 126]);
        assert_eq!(
            parse_key_name("S-Enter").unwrap().modifiers,
            KeyModifiers::SHIFT
        );
    }

    #[test]
    fn unknown_key_names_are_not_keys() {
        assert!(parse_key_name("hello").is_none());
        assert!(parse_key_name("F99").is_none());
        assert!(parse_key_name("C-").is_none());
        assert!(parse_key_name("✳ok").is_none());
    }

    #[test]
    fn unmapped_key_is_empty() {
        assert!(encode_key(&key(KeyCode::F(20), KeyModifiers::NONE), 0).is_empty());
    }
}
