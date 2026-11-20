//! Command blocks. The shell wraps its prompt in an OSC 8 link `rustty:<exit status of the previous
//! command>` (see shell/rustty.zsh). `Term` keeps links per cell, so prompts can be found by scanning
//! the grid. (OSC 133 would be cleaner, but alacritty_terminal drops it.)
//! ponytail: assumes the typed command fits on the prompt's last row.
use alacritty_terminal::{
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point},
    term::Term,
};

pub struct Prompt {
    pub first: i32,
    pub last: i32,
}

pub fn status_of(uri: &str) -> Option<i32> {
    uri.strip_prefix("rustty:")?.parse().ok()
}

/// All prompts, oldest first, as runs of consecutive rows carrying the link.
pub fn prompts<T>(t: &Term<T>) -> Vec<Prompt> {
    let mut out: Vec<Prompt> = Vec::new();
    for l in t.topmost_line().0..=t.bottommost_line().0 {
        let row = &t.grid()[Line(l)];
        let is_prompt = (0..t.columns()).any(|c| row[Column(c)].hyperlink().is_some_and(|h| status_of(h.uri()).is_some()));
        match (is_prompt, out.last_mut()) {
            (true, Some(p)) if p.last == l - 1 => p.last = l,
            (true, _) => out.push(Prompt { first: l, last: l }),
            _ => {}
        }
    }
    out
}

/// Output of the last finished command: rows between the last two prompts.
pub fn last_output<T>(t: &Term<T>) -> Option<String> {
    let p = prompts(t);
    let [.., a, b] = p.as_slice() else { return None };
    let (from, to) = (a.last + 1, b.first - 1);
    if to < from {
        return None;
    }
    let s = t.bounds_to_string(Point::new(Line(from), Column(0)), Point::new(Line(to), Column(t.columns() - 1)));
    Some(s.trim_end().to_string()).filter(|s| !s.is_empty())
}

/// Scroll so the previous (`up`) or next prompt sits at the top of the view.
pub fn jump<T: alacritty_terminal::event::EventListener>(t: &mut Term<T>, up: bool) {
    let off = t.grid().display_offset() as i32;
    let top = -off;
    let p = prompts(t);
    let target = if up {
        p.iter().rev().find(|p| p.first < top)
    } else {
        p.iter().find(|p| p.first > top)
    };
    match target {
        Some(p) => t.scroll_display(Scroll::Delta((-p.first).clamp(0, -t.topmost_line().0) - off)),
        None if !up => t.scroll_display(Scroll::Bottom),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::{event::VoidListener, term::Config, vte::ansi::Processor};

    struct Size;
    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            5
        }
        fn screen_lines(&self) -> usize {
            5
        }
        fn columns(&self) -> usize {
            20
        }
    }

    fn feed(s: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config { scrolling_history: 100, ..Default::default() }, &Size, VoidListener);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::default().advance(&mut t, s.as_bytes());
        t
    }

    fn prompt(status: i32) -> String {
        format!("\x1b]8;;rustty:{status}\x1b\\$ \x1b]8;;\x1b\\")
    }

    #[test]
    fn finds_prompts_with_status_and_last_output() {
        // 12 lines overflow the 5-row screen, so the first prompt lands in scrollback
        let t = feed(&format!("{}ls\r\nout1\r\nout2\r\n{}false\r\nboom\r\n{}", prompt(0), prompt(0), prompt(1)));
        let p = prompts(&t);
        assert_eq!(p.len(), 3);
        assert_eq!((status_of("rustty:1"), status_of("https://x")), (Some(1), None));
        assert_eq!(last_output(&t).as_deref(), Some("boom"));
    }

    #[test]
    fn jump_up_then_down_returns_to_bottom() {
        let mut t = feed(&format!("{}a\r\n1\r\n2\r\n{}b\r\n3\r\n4\r\n{}", prompt(0), prompt(0), prompt(0)));
        jump(&mut t, true);
        assert!(t.grid().display_offset() > 0);
        jump(&mut t, false);
        jump(&mut t, false);
        assert_eq!(t.grid().display_offset(), 0);
    }
}
