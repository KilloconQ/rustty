mod blocks;
mod config;
mod input;

use alacritty_terminal::{
    event::{Event as TermEvent, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{cell::Flags, color::Colors, Config as TermConfig, Term, TermMode},
    tty,
    vte::ansi::{Color, CursorShape, Rgb},
};
use config::{Config, Palette};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Event, FontData, FontDefinitions, FontFamily, FontId, Key,
    Pos2, Rect, Stroke, StrokeKind, Vec2, ViewportCommand,
};
use notify::Watcher;
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

/// Forwards terminal events to the UI thread and wakes it up.
#[derive(Clone)]
struct Proxy {
    ctx: egui::Context,
    tx: mpsc::Sender<TermEvent>,
}

impl EventListener for Proxy {
    fn send_event(&self, e: TermEvent) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

fn to32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn dim(c: Rgb) -> Rgb {
    let f = |v: u8| (v as u16 * 2 / 3) as u8;
    Rgb { r: f(c.r), g: f(c.g), b: f(c.b) }
}

/// Default color for a terminal color slot: 0-15 ansi, 16-255 cube/gray, 256+ named.
fn default_at(pal: &Palette, i: usize) -> Rgb {
    match i {
        0..=15 => pal.ansi[i],
        16..=231 => {
            let v = (i - 16) as u8;
            let l = |x: u8| if x == 0 { 0 } else { 55 + 40 * x };
            Rgb { r: l(v / 36), g: l(v / 6 % 6), b: l(v % 6) }
        }
        232..=255 => {
            let g = 8 + 10 * (i - 232) as u8;
            Rgb { r: g, g, b: g }
        }
        257 => pal.bg,
        258 => pal.cursor,
        259..=266 => dim(pal.ansi[i - 259]),
        268 => dim(pal.fg),
        _ => pal.fg, // 256 Foreground, 267 BrightForeground
    }
}

fn resolve(c: Color, colors: &Colors, pal: &Palette) -> Rgb {
    let i = match c {
        Color::Spec(rgb) => return rgb,
        Color::Indexed(i) => i as usize,
        Color::Named(n) => n as usize,
    };
    colors[i].unwrap_or_else(|| default_at(pal, i))
}

fn install_font(ctx: &egui::Context, file: &str) -> Result<(), String> {
    let mut fd = FontDefinitions::default();
    fd.font_data.insert("term".into(), Arc::new(FontData::from_owned(config::font_bytes(file)?)));
    let mono = fd.families.entry(FontFamily::Monospace).or_default();
    mono.insert(0, "term".into());
    // glyphs the Nerd Font lacks (e.g. dingbats like the starship error mark)
    if let Ok(b) = std::fs::read("/System/Library/Fonts/Menlo.ttc") {
        mono.insert(1, "symbols".into());
        fd.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(b)));
    }
    ctx.set_fonts(fd);
    Ok(())
}

/// One drawable cell, collected under the term lock so painting happens lock-free.
struct Drawn {
    col: usize,
    row: i32,
    ch: char,
    fg: Rgb,
    bg: Option<Rgb>,
    flags: Flags,
}

struct App {
    ctx: egui::Context,
    term: Arc<FairMutex<Term<Proxy>>>,
    pty: EventLoopSender,
    events: mpsc::Receiver<TermEvent>,
    reload_rx: mpsc::Receiver<()>,
    _watcher: Option<notify::RecommendedWatcher>,
    cfg: Config,
    pal: Palette,
    error: Option<String>,
    font_size: f32,
    cell: Vec2,
    grid: (usize, usize), // cols, rows
    focused: bool,
    preedit: String,
    toast: Option<Instant>, // when "Copied" was shown
    // mouse state
    dragging: bool,
    moved: bool,
    clicks: (Instant, (usize, usize), u8),
    held: Option<u8>,
    last_cell: (usize, usize),
    scroll_acc: f32,
}

impl App {
    fn new(ctx: egui::Context, cfg: Config, pal: Palette, mut error: Option<String>) -> Self {
        if let Err(e) = install_font(&ctx, &cfg.font_file) {
            error = Some(e);
        }
        let (etx, events) = mpsc::channel();
        let proxy = Proxy { ctx: ctx.clone(), tx: etx };
        let tcfg = TermConfig { scrolling_history: cfg.scrollback, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(tcfg, &Size { cols: 80, rows: 24 }, proxy.clone())));
        let mut argv: Vec<String> = std::env::args().skip(1).collect();
        if argv.first().is_some_and(|a| a == "-e") {
            argv.remove(0);
        }
        let opts = tty::Options {
            shell: (!argv.is_empty()).then(|| tty::Shell::new(argv.remove(0), argv.clone())),
            working_directory: dirs::home_dir(),
            env: HashMap::from([
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ]),
            ..Default::default()
        };
        let ws = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let pty = tty::new(&opts, ws, 0).expect("cannot open pty");
        let el = EventLoop::new(term.clone(), proxy, pty, false, false).expect("cannot start pty loop");
        let sender = el.channel();
        el.spawn();

        let (wtx, reload_rx) = mpsc::channel();
        let wctx = ctx.clone();
        let watcher = notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
            if r.is_ok() {
                let _ = wtx.send(());
                wctx.request_repaint();
            }
        })
        .ok()
        .and_then(|mut w| {
            w.watch(&config::dir(), notify::RecursiveMode::Recursive).ok()?;
            Some(w)
        });

        Self {
            ctx,
            term,
            pty: sender,
            events,
            reload_rx,
            _watcher: watcher,
            font_size: cfg.font_size,
            cfg,
            pal,
            error,
            cell: vec2(8.0, 16.0),
            grid: (80, 24),
            focused: true,
            preedit: String::new(),
            toast: None,
            dragging: false,
            moved: false,
            clicks: (Instant::now() - Duration::from_secs(10), (0, 0), 0),
            held: None,
            last_cell: (usize::MAX, 0),
            scroll_acc: 0.0,
        }
    }

    fn reload(&mut self) {
        match config::load() {
            Ok((cfg, pal)) => {
                if cfg.font_file != self.cfg.font_file {
                    if let Err(e) = install_font(&self.ctx, &cfg.font_file) {
                        self.error = Some(e);
                        return;
                    }
                }
                if cfg.opacity != self.cfg.opacity {
                    self.ctx.send_viewport_cmd(ViewportCommand::Transparent(cfg.opacity < 1.0));
                }
                self.term.lock().set_options(TermConfig {
                    scrolling_history: cfg.scrollback,
                    ..Default::default()
                });
                self.font_size = cfg.font_size;
                self.cfg = cfg;
                self.pal = pal;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.pty.send(Msg::Input(bytes.into()));
    }

    /// Keyboard/paste input: jump to the bottom and drop any selection, like other terminals.
    fn type_bytes(&self, bytes: Vec<u8>) {
        {
            let mut t = self.term.lock();
            t.scroll_display(Scroll::Bottom);
            t.selection = None;
        }
        self.write(bytes);
    }

    fn drain_term_events(&mut self, ctx: &egui::Context) {
        let events: Vec<_> = self.events.try_iter().collect();
        for e in events {
            match e {
                TermEvent::PtyWrite(s) => self.write(s.into_bytes()),
                TermEvent::Title(t) => ctx.send_viewport_cmd(ViewportCommand::Title(t)),
                TermEvent::ResetTitle => ctx.send_viewport_cmd(ViewportCommand::Title("rustty".into())),
                TermEvent::ClipboardStore(_, s) => ctx.copy_text(s),
                TermEvent::ColorRequest(i, fmt) => {
                    let rgb = {
                        let t = self.term.lock();
                        let colors = t.renderable_content().colors;
                        colors[i].unwrap_or_else(|| default_at(&self.pal, i))
                    };
                    self.write(fmt(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(fmt) => {
                    let ws = WindowSize {
                        num_lines: self.grid.1 as u16,
                        num_cols: self.grid.0 as u16,
                        cell_width: self.cell.x as u16,
                        cell_height: self.cell.y as u16,
                    };
                    self.write(fmt(ws).into_bytes());
                }
                TermEvent::Exit | TermEvent::ChildExit(_) => ctx.send_viewport_cmd(ViewportCommand::Close),
                _ => {}
            }
        }
    }

    fn cell_at(&self, rect: Rect, pos: Pos2) -> (usize, usize, Side) {
        let x = (pos.x - rect.min.x).max(0.0) / self.cell.x;
        let y = (pos.y - rect.min.y).max(0.0) / self.cell.y;
        let col = (x as usize).min(self.grid.0 - 1);
        let row = (y as usize).min(self.grid.1 - 1);
        (col, row, if x.fract() < 0.5 { Side::Left } else { Side::Right })
    }

    fn point(&self, col: usize, row: usize) -> Point {
        let off = self.term.lock().grid().display_offset() as i32;
        Point::new(Line(row as i32 - off), Column(col))
    }

    fn copy_selection(&mut self) {
        let text = self.term.lock().selection_to_string().filter(|s| !s.is_empty());
        if let Some(s) = text {
            self.ctx.copy_text(s);
            self.toast = Some(Instant::now());
        }
    }

    fn shortcut(&mut self, ctx: &egui::Context, key: Key, m: egui::Modifiers) {
        match key {
            Key::Q => ctx.send_viewport_cmd(ViewportCommand::Close),
            Key::Plus | Key::Equals => self.font_size = (self.font_size + 1.0).min(72.0),
            Key::Minus => self.font_size = (self.font_size - 1.0).max(6.0),
            Key::Num0 => self.font_size = self.cfg.font_size,
            Key::Comma if m.shift => self.reload(),
            Key::W => ctx.send_viewport_cmd(ViewportCommand::Close),
            // command blocks: jump between prompts, copy the last command's output
            Key::ArrowUp => blocks::jump(&mut self.term.lock(), true),
            Key::ArrowDown => blocks::jump(&mut self.term.lock(), false),
            Key::O if m.shift => {
                let out = blocks::last_output(&self.term.lock());
                if let Some(s) = out {
                    ctx.copy_text(s);
                    self.toast = Some(Instant::now());
                }
            }
            // line editing, like Terminal/iTerm: kill line, start/end of line
            Key::Backspace => self.type_bytes(vec![0x15]),
            Key::ArrowLeft => self.type_bytes(vec![0x01]),
            Key::ArrowRight => self.type_bytes(vec![0x05]),
            _ => {}
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context, rect: Rect) {
        let events = ctx.input(|i| i.events.clone());
        let mods = ctx.input(|i| i.modifiers);
        let mode = *self.term.lock().mode();
        let sgr = mode.contains(TermMode::SGR_MOUSE);
        let mut committed = String::new(); // Text duplicating an IME commit in the same frame

        for ev in events {
            match ev {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if modifiers.command {
                        self.shortcut(ctx, key, modifiers);
                    } else if let Some(b) = input::key_bytes(
                        key,
                        modifiers,
                        mode.contains(TermMode::APP_CURSOR),
                        self.cfg.option_as_alt,
                    ) {
                        self.type_bytes(b);
                    }
                }
                Event::Text(s) => {
                    let alt_as_meta = self.cfg.option_as_alt && mods.alt;
                    if !alt_as_meta && !mods.command && s != committed {
                        self.type_bytes(s.into_bytes());
                    }
                }
                Event::Ime(egui::ImeEvent::Preedit { text, .. }) => self.preedit = text,
                Event::Ime(egui::ImeEvent::Commit(s)) => {
                    self.preedit.clear();
                    committed = s.clone();
                    self.type_bytes(s.into_bytes());
                }
                Event::Ime(_) => self.preedit.clear(),
                Event::Paste(s) => {
                    let s = s.replace("\r\n", "\r").replace('\n', "\r");
                    let bytes = if mode.contains(TermMode::BRACKETED_PASTE) {
                        format!("\x1b[200~{}\x1b[201~", s.replace('\x1b', ""))
                    } else {
                        s
                    };
                    self.type_bytes(bytes.into_bytes());
                }
                Event::Copy => self.copy_selection(),
                Event::WindowFocused(f) => {
                    self.focused = f;
                    if mode.contains(TermMode::FOCUS_IN_OUT) {
                        self.write(if f { b"\x1b[I".as_slice() } else { b"\x1b[O".as_slice() }.to_vec());
                    }
                }
                Event::PointerButton { pos, button, pressed, modifiers } => {
                    let (col, row, side) = self.cell_at(rect, pos);
                    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift;
                    if pressed && !rect.expand(self.cfg.padding).contains(pos) {
                        continue;
                    }
                    if reporting {
                        let b = match button {
                            egui::PointerButton::Primary => 0,
                            egui::PointerButton::Middle => 1,
                            egui::PointerButton::Secondary => 2,
                            _ => continue,
                        } | input::mouse_mods(modifiers);
                        self.write(input::mouse_report(sgr, b, pressed, col, row));
                        self.held = pressed.then_some(b);
                    } else if button == egui::PointerButton::Primary {
                        if !pressed {
                            self.finish_selection();
                        } else if modifiers.command {
                            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                        } else {
                            self.start_selection(col, row, side);
                        }
                    }
                }
                Event::PointerMoved(pos) => {
                    let (col, row, side) = self.cell_at(rect, pos);
                    if mode.intersects(TermMode::MOUSE_MODE) && !mods.shift {
                        let drag = mode.contains(TermMode::MOUSE_DRAG) && self.held.is_some();
                        if (mode.contains(TermMode::MOUSE_MOTION) || drag) && self.last_cell != (col, row) {
                            self.last_cell = (col, row);
                            let b = self.held.unwrap_or(3) + 32;
                            self.write(input::mouse_report(sgr, b, true, col, row));
                        }
                    } else if self.dragging {
                        let p = self.point(col, row);
                        if let Some(sel) = self.term.lock().selection.as_mut() {
                            sel.update(p, side);
                        }
                        self.moved = true;
                    }
                }
                Event::MouseWheel { unit, delta, modifiers, .. } => {
                    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { continue };
                    let dy = match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * self.cell.y,
                        egui::MouseWheelUnit::Page => delta.y * self.cell.y * self.grid.1 as f32,
                    };
                    self.scroll_acc += dy;
                    let n = (self.scroll_acc / self.cell.y).trunc() as i32;
                    self.scroll_acc -= n as f32 * self.cell.y;
                    if n == 0 {
                        continue;
                    }
                    let (col, row, _) = self.cell_at(rect, pos);
                    if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
                        let b = (if n > 0 { 64 } else { 65 }) | input::mouse_mods(modifiers);
                        for _ in 0..n.abs() {
                            self.write(input::mouse_report(sgr, b, true, col, row));
                        }
                    } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                        let k = if n > 0 { 'A' } else { 'B' };
                        let seq = if mode.contains(TermMode::APP_CURSOR) {
                            format!("\x1bO{k}")
                        } else {
                            format!("\x1b[{k}")
                        };
                        self.write(seq.repeat(n.unsigned_abs() as usize).into_bytes());
                    } else {
                        self.term.lock().scroll_display(Scroll::Delta(n));
                    }
                }
                _ => {}
            }
        }
    }

    fn start_selection(&mut self, col: usize, row: usize, side: Side) {
        let (t, last, n) = self.clicks;
        let n = if t.elapsed() < Duration::from_millis(400) && last == (col, row) { n % 3 + 1 } else { 1 };
        self.clicks = (Instant::now(), (col, row), n);
        let ty = match n {
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        let p = self.point(col, row);
        self.term.lock().selection = Some(Selection::new(ty, p, side));
        self.dragging = true;
        self.moved = false;
    }

    fn finish_selection(&mut self) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        if self.clicks.2 == 1 && !self.moved {
            self.term.lock().selection = None; // plain click: no selection
        } else if self.cfg.copy_on_select {
            self.copy_selection();
        }
    }

    fn paint(&mut self, ui: &egui::Ui, full: Rect, rect: Rect, font: &FontId) {
        let pal = self.pal;
        let painter = ui.painter_at(full);
        let alpha = (self.cfg.opacity.clamp(0.0, 1.0) * 255.0) as u8;
        painter.rect_filled(full, 0.0, Color32::from_rgba_unmultiplied(pal.bg.r, pal.bg.g, pal.bg.b, alpha));

        let (cw, ch) = (self.cell.x, self.cell.y);
        let (mut cells, cursor_at, shape, cursor_rgb);
        let mut failed: Vec<i32> = Vec::new(); // visible prompt rows whose previous command failed
        {
            let t = self.term.lock();
            let c = t.renderable_content();
            let off = c.display_offset as i32;
            cursor_rgb = c.colors[258usize].unwrap_or(pal.cursor);
            shape = if !c.mode.contains(TermMode::SHOW_CURSOR) {
                CursorShape::Hidden
            } else if !self.focused && c.cursor.shape != CursorShape::Hidden {
                CursorShape::HollowBlock
            } else {
                c.cursor.shape
            };
            cursor_at = (c.cursor.point.column.0, c.cursor.point.line.0 + off);
            cells = Vec::new();
            for it in c.display_iter {
                let cell = it.cell;
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                let mut fg = resolve(cell.fg, c.colors, &pal);
                let mut bg = resolve(cell.bg, c.colors, &pal);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.flags.contains(Flags::DIM) {
                    fg = dim(fg);
                }
                if c.selection.is_some_and(|s| s.contains(it.point)) {
                    bg = pal.selection;
                }
                let (col, row) = (it.point.column.0, it.point.line.0 + off);
                if shape == CursorShape::Block && (col, row) == cursor_at {
                    (fg, bg) = (pal.bg, cursor_rgb);
                }
                if row != failed.last().copied().unwrap_or(-1)
                    && cell.hyperlink().and_then(|h| blocks::status_of(h.uri())).is_some_and(|s| s != 0)
                {
                    failed.push(row);
                }
                let blank = cell.c == ' ' || cell.c == '\0';
                let decorated = cell.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
                if blank && bg == pal.bg && !decorated {
                    continue;
                }
                cells.push(Drawn {
                    col,
                    row,
                    ch: if blank || cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c },
                    fg,
                    bg: (bg != pal.bg).then_some(bg),
                    flags: cell.flags,
                });
            }
        }

        for &r in &failed {
            let y = rect.min.y + r as f32 * ch;
            painter.rect_filled(Rect::from_min_size(pos2(full.min.x, y), vec2(2.0, ch)), 0.0, to32(pal.ansi[1]));
        }

        for d in &cells {
            let (x, y) = (rect.min.x + d.col as f32 * cw, rect.min.y + d.row as f32 * ch);
            let w = if d.flags.contains(Flags::WIDE_CHAR) { 2.0 * cw } else { cw };
            if let Some(bg) = d.bg {
                painter.rect_filled(Rect::from_min_size(pos2(x, y), vec2(w, ch)), 0.0, to32(bg));
            }
            let fg = to32(d.fg);
            if d.ch != ' ' {
                // ponytail: one painter.text per cell (egui caches galleys); batch runs if profiling says so
                painter.text(pos2(x, y), Align2::LEFT_TOP, d.ch, font.clone(), fg);
                if d.flags.contains(Flags::BOLD) {
                    painter.text(pos2(x + 0.6, y), Align2::LEFT_TOP, d.ch, font.clone(), fg); // faux bold
                }
            }
            if d.flags.intersects(Flags::ALL_UNDERLINES) {
                painter.hline(x..=x + w, y + ch - 1.5, Stroke::new(1.0, fg));
            }
            if d.flags.contains(Flags::STRIKEOUT) {
                painter.hline(x..=x + w, y + ch / 2.0, Stroke::new(1.0, fg));
            }
        }

        let cur = Rect::from_min_size(
            pos2(rect.min.x + cursor_at.0 as f32 * cw, rect.min.y + cursor_at.1 as f32 * ch),
            vec2(cw, ch),
        );
        let ccol = to32(cursor_rgb);
        match shape {
            CursorShape::HollowBlock => {
                painter.rect_stroke(cur.shrink(0.5), 0.0, Stroke::new(1.0, ccol), StrokeKind::Inside);
            }
            CursorShape::Underline => {
                painter.rect_filled(Rect::from_min_size(pos2(cur.min.x, cur.max.y - 2.0), vec2(cw, 2.0)), 0.0, ccol);
            }
            CursorShape::Beam => {
                painter.rect_filled(Rect::from_min_size(cur.min, vec2(2.0, ch)), 0.0, ccol);
            }
            _ => {}
        }

        if !self.preedit.is_empty() {
            let g = painter.layout_no_wrap(self.preedit.clone(), font.clone(), to32(pal.fg));
            let r = Rect::from_min_size(cur.min, g.size());
            painter.rect_filled(r, 0.0, to32(pal.selection));
            painter.galley(cur.min, g, to32(pal.fg));
            painter.hline(r.x_range(), r.max.y - 1.0, Stroke::new(1.0, to32(pal.fg)));
        }

        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect,
                cursor_rect: cur,
                should_interrupt_composition: false,
            })
        });

        const TOAST: Duration = Duration::from_millis(1400);
        if let Some(t) = self.toast {
            if t.elapsed() >= TOAST {
                self.toast = None;
            } else {
                let g = painter.layout_no_wrap("Copied to clipboard".into(), FontId::monospace(12.0), Color32::WHITE);
                let pos = full.right_top() + vec2(-12.0 - g.size().x, 12.0);
                painter.rect_filled(Rect::from_min_size(pos, g.size()).expand(6.0), 6.0, Color32::from_black_alpha(220));
                painter.galley(pos, g, Color32::WHITE);
                ui.ctx().request_repaint_after(TOAST - t.elapsed()); // redraw once to clear it
            }
        }

        if let Some(e) = &self.error {
            let g = painter.layout_no_wrap(format!("config: {e}"), FontId::monospace(12.0), Color32::from_rgb(255, 140, 140));
            let pos = full.left_bottom() + vec2(8.0, -8.0 - g.size().y);
            painter.rect_filled(Rect::from_min_size(pos, g.size()).expand(4.0), 3.0, Color32::from_black_alpha(220));
            painter.galley(pos, g, Color32::WHITE);
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // the terminal paints its own (possibly translucent) background
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.reload_rx.try_iter().count() > 0 {
            self.reload();
        }
        self.drain_term_events(&ctx);

        let full = ui.max_rect();
        let rect = full.shrink(self.cfg.padding);
        let font = FontId::monospace(self.font_size);
        let ppp = ctx.pixels_per_point();
        let snap = |v: f32| ((v * ppp).round() / ppp).max(1.0 / ppp);
        let (cw, ch) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        self.cell = vec2(snap(cw), snap(ch));

        let cols = ((rect.width() / self.cell.x) as usize).max(2);
        let rows = ((rect.height() / self.cell.y) as usize).max(1);
        if (cols, rows) != self.grid {
            self.grid = (cols, rows);
            self.term.lock().resize(Size { cols, rows });
            let _ = self.pty.send(Msg::Resize(WindowSize {
                num_lines: rows as u16,
                num_cols: cols as u16,
                cell_width: self.cell.x as u16,
                cell_height: self.cell.y as u16,
            }));
        }

        self.handle_input(&ctx, rect);
        self.paint(ui, full, rect, &font);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.pty.send(Msg::Shutdown);
    }
}

fn main() -> eframe::Result {
    let (cfg, pal, err) = match config::load() {
        Ok((c, p)) => (c, p, None),
        Err(e) => {
            let (c, p) = config::defaults();
            (c, p, Some(e))
        }
    };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // a real titled window (AeroSpace only tiles standard windows), drawn bare
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(false)
            .with_transparent(cfg.opacity < 1.0)
            .with_inner_size([960.0, 600.0])
            .with_title("rustty"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "rustty",
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc.egui_ctx.clone(), cfg, pal, err)))),
    )
}
