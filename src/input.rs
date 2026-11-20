//! Pure encoders: egui keys/mouse -> bytes for the PTY.
use eframe::egui::{Key, Modifiers};

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn mod_param(m: Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8
}

/// CSI/SS3 sequence with an optional modifier: `ESC [ 1 ; m X`.
fn csi(m: Modifiers, app: bool, fin: char) -> Vec<u8> {
    match (mod_param(m), app) {
        (1, true) => format!("\x1bO{fin}"),
        (1, false) => format!("\x1b[{fin}"),
        (p, _) => format!("\x1b[1;{p}{fin}"),
    }
    .into_bytes()
}

fn tilde(m: Modifiers, n: u8) -> Vec<u8> {
    match mod_param(m) {
        1 => format!("\x1b[{n}~"),
        p => format!("\x1b[{n};{p}~"),
    }
    .into_bytes()
}

/// Bytes for a non-text key press. Plain text comes through `Event::Text`, not here.
/// Returns None for keys that produce text or for Cmd shortcuts (handled by the app).
pub fn key_bytes(key: Key, m: Modifiers, app_cursor: bool, option_as_alt: bool) -> Option<Vec<u8>> {
    if m.command {
        return None;
    }
    let name = key.name();
    let letter = (name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic())
        .then(|| name.as_bytes()[0].to_ascii_lowercase());

    let bytes: Vec<u8> = match key {
        Key::Enter => b"\r".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace if m.ctrl => b"\x08".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        // Option+Left/Right: word movement, the way readline/zsh expect it
        Key::ArrowLeft if m.alt && !m.shift && !m.ctrl => b"\x1bb".to_vec(),
        Key::ArrowRight if m.alt && !m.shift && !m.ctrl => b"\x1bf".to_vec(),
        Key::ArrowUp => return Some(csi(m, app_cursor, 'A')),
        Key::ArrowDown => return Some(csi(m, app_cursor, 'B')),
        Key::ArrowRight => return Some(csi(m, app_cursor, 'C')),
        Key::ArrowLeft => return Some(csi(m, app_cursor, 'D')),
        Key::Home => return Some(csi(m, app_cursor, 'H')),
        Key::End => return Some(csi(m, app_cursor, 'F')),
        Key::Insert => return Some(tilde(m, 2)),
        Key::Delete => return Some(tilde(m, 3)),
        Key::PageUp => return Some(tilde(m, 5)),
        Key::PageDown => return Some(tilde(m, 6)),
        Key::F1 => return Some(csi(m, true, 'P')),
        Key::F2 => return Some(csi(m, true, 'Q')),
        Key::F3 => return Some(csi(m, true, 'R')),
        Key::F4 => return Some(csi(m, true, 'S')),
        Key::F5 => return Some(tilde(m, 15)),
        Key::F6 => return Some(tilde(m, 17)),
        Key::F7 => return Some(tilde(m, 18)),
        Key::F8 => return Some(tilde(m, 19)),
        Key::F9 => return Some(tilde(m, 20)),
        Key::F10 => return Some(tilde(m, 21)),
        Key::F11 => return Some(tilde(m, 23)),
        Key::F12 => return Some(tilde(m, 24)),
        Key::Space if m.ctrl => vec![0],
        Key::OpenBracket if m.ctrl => vec![0x1b],
        Key::Backslash if m.ctrl => vec![0x1c],
        Key::CloseBracket if m.ctrl => vec![0x1d],
        _ => match letter {
            Some(l) if m.ctrl => vec![l - b'a' + 1],
            Some(l) if m.alt && option_as_alt => {
                vec![if m.shift { l.to_ascii_uppercase() } else { l }]
            }
            _ => return None,
        },
    };
    let word_move = matches!(key, Key::ArrowLeft | Key::ArrowRight);
    let alt_prefix = m.alt && !word_move && (option_as_alt || letter.is_none());
    Some(if alt_prefix { [b"\x1b".as_slice(), &bytes].concat() } else { bytes })
}

/// Modifier bits added to a mouse button code (shift 4, alt 8, ctrl 16).
pub fn mouse_mods(m: Modifiers) -> u8 {
    4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.ctrl as u8
}

/// Mouse report. `button` already includes modifier/motion bits. col/row are 0-based.
pub fn mouse_report(sgr: bool, button: u8, press: bool, col: usize, row: usize) -> Vec<u8> {
    if sgr {
        let end = if press { 'M' } else { 'm' };
        return format!("\x1b[<{button};{};{}{end}", col + 1, row + 1).into_bytes();
    }
    let b = if press { button } else { (button & !3) | 3 };
    // legacy encoding cannot express coordinates past 222
    let (Ok(x), Ok(y)) = (u8::try_from(33 + col), u8::try_from(33 + row)) else {
        return vec![];
    };
    vec![0x1b, b'[', b'M', 32 + b, x, y]
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const CTRL: Modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };
    const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..Modifiers::NONE };

    fn k(key: Key, m: Modifiers, app: bool) -> Option<Vec<u8>> {
        key_bytes(key, m, app, false)
    }

    #[test]
    fn basic_keys() {
        assert_eq!(k(Key::Enter, NONE, false).unwrap(), b"\r");
        assert_eq!(k(Key::Backspace, NONE, false).unwrap(), b"\x7f");
        assert_eq!(k(Key::Tab, SHIFT, false).unwrap(), b"\x1b[Z");
        assert_eq!(k(Key::C, CTRL, false).unwrap(), vec![3]);
        assert_eq!(k(Key::L, CTRL, false).unwrap(), vec![12]);
    }

    #[test]
    fn arrows_follow_app_cursor_and_modifiers() {
        assert_eq!(k(Key::ArrowUp, NONE, false).unwrap(), b"\x1b[A");
        assert_eq!(k(Key::ArrowUp, NONE, true).unwrap(), b"\x1bOA");
        assert_eq!(k(Key::ArrowLeft, SHIFT, false).unwrap(), b"\x1b[1;2D");
        assert_eq!(k(Key::ArrowLeft, ALT, true).unwrap(), b"\x1bb"); // word back
        assert_eq!(k(Key::ArrowRight, ALT, false).unwrap(), b"\x1bf");
        assert_eq!(k(Key::Delete, NONE, false).unwrap(), b"\x1b[3~");
        assert_eq!(k(Key::F5, CTRL, false).unwrap(), b"\x1b[15;5~");
    }

    #[test]
    fn plain_letters_are_text_not_keys() {
        assert_eq!(k(Key::A, NONE, false), None);
        // Option+e must stay a text/IME event so accents work
        assert_eq!(k(Key::E, ALT, false), None);
        // with option_as_alt it becomes ESC e
        assert_eq!(key_bytes(Key::E, ALT, false, true).unwrap(), b"\x1be");
    }

    #[test]
    fn cmd_shortcuts_are_left_to_the_app() {
        let cmd = Modifiers { command: true, ..Modifiers::NONE };
        assert_eq!(k(Key::C, cmd, false), None);
    }

    #[test]
    fn mouse_encodings() {
        assert_eq!(mouse_report(true, 0, true, 0, 0), b"\x1b[<0;1;1M");
        assert_eq!(mouse_report(true, 0, false, 9, 4), b"\x1b[<0;10;5m");
        assert_eq!(mouse_report(false, 0, true, 0, 0), vec![0x1b, b'[', b'M', 32, 33, 33]);
        assert_eq!(mouse_report(false, 0, false, 0, 0), vec![0x1b, b'[', b'M', 35, 33, 33]);
        assert!(mouse_report(false, 0, true, 300, 0).is_empty());
    }
}
