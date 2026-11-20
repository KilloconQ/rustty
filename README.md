# rustty

A minimal, borderless, GPU-accelerated terminal for macOS, in pure Rust.
`alacritty_terminal` (PTY, VT parser, grid) rendered with `egui` on wgpu (Metal).

- Nerd Font support, borderless window, copy-on-select, IME/accents
- tmux/zellij friendly: mouse reporting, truecolor, bracketed paste
- TOML config with live reload and built-in themes (`catppuccin-latte`, `dracula`, `gruvbox-dark`, `nord`, `one-dark`, `solarized-dark`, `solarized-light`, `tokyo-night`)

## Run

```sh
cargo run --release
```

Config lives in `~/.config/rustty/config.toml` (created on first run). Set `theme = "dracula"` to pick a built-in theme, or drop your own in `~/.config/rustty/themes/<name>.toml`.

Optional: `source shell/rustty.zsh` last in `~/.zshrc` to enable command-block navigation.

## License

MIT
