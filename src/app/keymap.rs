//! Pure translation from crossterm key events to the byte sequences a PTY
//! expects. No app state — easy to read and to unit-test.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode a crossterm key event into the bytes to send to the PTY. Returns an
/// empty vec for keys mmux doesn't forward.
pub fn encode_key(k: &KeyEvent) -> Vec<u8> {
    use KeyCode::*;
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);

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
        Enter => vec![b'\r'],
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_char() {
        assert_eq!(encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE)), b"a");
    }

    #[test]
    fn ctrl_c_is_0x03() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            vec![0x03]
        );
    }

    #[test]
    fn ctrl_b_is_0x02() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            vec![0x02]
        );
    }

    #[test]
    fn enter_is_carriage_return() {
        assert_eq!(encode_key(&key(KeyCode::Enter, KeyModifiers::NONE)), b"\r");
    }

    #[test]
    fn backspace_is_del() {
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE)),
            vec![0x7f]
        );
    }

    #[test]
    fn arrows_are_csi() {
        assert_eq!(encode_key(&key(KeyCode::Up, KeyModifiers::NONE)), vec![27, 91, 65]);
        assert_eq!(encode_key(&key(KeyCode::Down, KeyModifiers::NONE)), vec![27, 91, 66]);
    }

    #[test]
    fn alt_prefixes_esc() {
        assert_eq!(encode_key(&key(KeyCode::Char('x'), KeyModifiers::ALT)), vec![27, b'x']);
    }

    #[test]
    fn unmapped_key_is_empty() {
        assert!(encode_key(&key(KeyCode::F(20), KeyModifiers::NONE)).is_empty());
    }
}
