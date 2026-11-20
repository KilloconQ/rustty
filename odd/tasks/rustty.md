# rustty — bare GPU terminal in Rust
Branch: none (not a git repo yet). Delivery: local. Runner: cargo test.

## Specs
S1 "con rust" — pure Rust: alacritty_terminal (PTY + parser + grid) + eframe/egui on wgpu (Metal).
S2 "que pueda correr con nerdfonts" — load a Nerd Font file from ~/Library/Fonts (default Hack Nerd Font Mono).
S3 "sin bordes" — borderless window, zero padding; Cmd+Q closes, Cmd+drag moves.
S4 "config toml" with live reload, colors/themes, font size ("para poder actualizar y cambiar colores").
S5 "acelerado por gui" — GPU accelerated (wgpu).
S6 tmux/zellij support: mouse reporting, truecolor, bracketed paste, cursor shapes.
S7 IME/accents: Option+e then a gives á.
S8 "iluminando, se copie al clipboard" — selecting text copies to clipboard (copy-on-select).
S9 Cmd+C / Cmd+V copy/paste; Cmd +/-/0 font size.

## Tasks
T1 S1,S5 core: pty + term + render grid — inline — done (launches, renders prompt, wgpu)
T2 S2,S3,S4,S9 font, borderless, config+reload+themes — inline — implemented; reload and Cmd shortcuts NOT exercised
T3 S6,S7,S8 input: keys, mouse, IME, selection — inline — implemented; unit tests for key+mouse encoders pass; interactive behavior NOT exercised

## Log
L1 (2026-11-20) user: "cuanto te tardarías en hacer una terminal como esta? sin tantas cosas fancys ponte la pura terminal" → then rust, nerdfonts, sin bordes, ghostty-like config reload, toml (not lua), GPU, tmux+IME, copy on select.
