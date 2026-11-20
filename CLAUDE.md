# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

`rustty` is a minimal, borderless, GPU-accelerated macOS terminal emulator in pure Rust: `alacritty_terminal` (PTY, VT parser, grid) rendered with `eframe`/`egui` on wgpu (Metal). Requirements and their status live in `odd/tasks/rustty.md` (specs S1–S9; interactive behavior such as config reload and Cmd shortcuts is noted there as not yet exercised).

## Commands

- Build/run: `cargo run` (release: `cargo run --release`)
- Tests: `cargo test`
- Single test: `cargo test <name>` (e.g. `cargo test mouse_encodings`)
- Lint: `cargo clippy`

Tests exist only for pure logic (`config.rs` merging/color parsing, `input.rs` key and mouse encoders). Rendering and interaction have no automated tests.

## Architecture

- `src/main.rs` — `App` (eframe app). Owns the `alacritty_terminal::Term` behind a `FairMutex`, the PTY `EventLoopSender`, and UI state. `Proxy` implements `EventListener`: terminal events go through an mpsc channel to the UI thread and call `request_repaint`. `paint` copies visible cells into `Drawn` structs under the term lock, then draws lock-free. Colors resolve via `resolve`/`default_at` (palette index layout: 0–15 ansi, 16–255 cube/gray, 256+ alacritty named colors).
- `src/input.rs` — pure encoders from egui keys/mouse to terminal bytes (app-cursor mode, modifier params, mouse reporting for tmux/zellij). Cmd shortcuts are intentionally not encoded; `App::shortcut` handles them.
- `src/config.rs` — TOML config at `~/.config/rustty/config.toml` (created from `DEFAULT` if missing). Layers are merged table-wise: built-in defaults ← optional theme (`~/.config/rustty/themes/<theme>.toml`) ← user file. `notify` watches the files and `App::reload` applies changes live; a bad config surfaces as `App.error` instead of crashing.
- Fonts: a Nerd Font file is searched in `~/Library/Fonts` and `/Library/Fonts` (default `HackNerdFontMono-Regular.ttf`), with `/System/Library/Fonts/Menlo.ttc` as a symbol fallback (`install_font`).

## Conventions

- Rust edition 2024.
- Initial commit on `main`; `odd/tasks/rustty.md` is the ODD feature document and should be updated as tasks progress.
