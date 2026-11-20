//! TOML config: defaults <- optional theme file <- user file, merged table-wise.
use alacritty_terminal::vte::ansi::Rgb;
use serde::Deserialize;
use std::path::PathBuf;
use toml::Table;

pub const DEFAULT: &str = r##"# rustty config. Saving this file applies changes live.
font_file = "HackNerdFontMono-Regular.ttf" # searched in ~/Library/Fonts, /Library/Fonts; or an absolute path
font_size = 14.0
opacity = 1.0
padding = 8.0 # points between the window edge and the text
copy_on_select = true
option_as_alt = false # true: Option sends Alt (disables Option+e accents)
scrollback = 10000
# theme = "dracula"   # built in: catppuccin-latte, dracula, gruvbox-dark, nord, one-dark,
#                     # solarized-dark, solarized-light, tokyo-night. Or your own
#                     # ~/.config/rustty/themes/name.toml (same [colors] table, wins over built-ins).
#                     # A [colors] table in this file overrides the theme, key by key.

[colors]
foreground = "#cdd6f4"
background = "#1e1e2e"
cursor = "#f5e0dc"
selection = "#45475a"
ansi = [
  "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de",
  "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8",
]
"##;

#[derive(Deserialize, Clone, PartialEq)]
pub struct Config {
    pub font_file: String,
    pub font_size: f32,
    pub opacity: f32,
    pub padding: f32,
    pub copy_on_select: bool,
    pub option_as_alt: bool,
    pub scrollback: usize,
    colors: Colors,
}

#[derive(Deserialize, Clone, PartialEq)]
struct Colors {
    foreground: String,
    background: String,
    cursor: String,
    selection: String,
    ansi: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

const THEMES: &[(&str, &str)] = &[
    ("catppuccin-latte", include_str!("themes/catppuccin-latte.toml")),
    ("dracula", include_str!("themes/dracula.toml")),
    ("gruvbox-dark", include_str!("themes/gruvbox-dark.toml")),
    ("nord", include_str!("themes/nord.toml")),
    ("one-dark", include_str!("themes/one-dark.toml")),
    ("solarized-dark", include_str!("themes/solarized-dark.toml")),
    ("solarized-light", include_str!("themes/solarized-light.toml")),
    ("tokyo-night", include_str!("themes/tokyo-night.toml")),
];

pub fn dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".config/rustty")
}

pub fn hex(s: &str) -> Result<Rgb, String> {
    let h = s.strip_prefix('#').unwrap_or(s);
    let v = (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten();
    let v = v.ok_or_else(|| format!("bad color {s:?}, expected #rrggbb"))?;
    Ok(Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 })
}

fn build(t: Table) -> Result<(Config, Palette), String> {
    let cfg: Config = t.try_into().map_err(|e| e.to_string())?;
    let c = &cfg.colors;
    if c.ansi.len() != 16 {
        return Err(format!("colors.ansi needs 16 entries, got {}", c.ansi.len()));
    }
    let mut ansi = [Rgb { r: 0, g: 0, b: 0 }; 16];
    for (slot, s) in ansi.iter_mut().zip(&c.ansi) {
        *slot = hex(s)?;
    }
    let pal = Palette {
        fg: hex(&c.foreground)?,
        bg: hex(&c.background)?,
        cursor: hex(&c.cursor)?,
        selection: hex(&c.selection)?,
        ansi,
    };
    Ok((cfg, pal))
}

fn merge(base: &mut Table, over: Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

pub fn defaults() -> (Config, Palette) {
    build(DEFAULT.parse().expect("DEFAULT is valid")).expect("DEFAULT is valid")
}

/// Reads ~/.config/rustty/config.toml (creating it from DEFAULT if missing).
pub fn load() -> Result<(Config, Palette), String> {
    let path = dir().join("config.toml");
    if !path.exists() {
        let _ = std::fs::create_dir_all(dir().join("themes"));
        // colors come from the default/theme; a [colors] table here would pin them over any theme
        let _ = std::fs::write(&path, DEFAULT.split("\n[colors]").next().unwrap_or(DEFAULT));
    }
    let read = |p: &PathBuf| -> Result<Table, String> {
        let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        s.parse().map_err(|e| format!("{}: {e}", p.display()))
    };
    // missing and not creatable (read-only HOME, etc.): run on defaults, no error
    let user = if path.exists() { read(&path)? } else { Table::new() };
    let mut t: Table = DEFAULT.parse().expect("DEFAULT is valid");
    if let Some(name) = user.get("theme").and_then(|v| v.as_str()) {
        let file = dir().join("themes").join(format!("{name}.toml"));
        let theme = match THEMES.iter().find(|(n, _)| n == &name) {
            Some((_, s)) if !file.exists() => s.parse().expect("built-in theme is valid"),
            _ => read(&file)?,
        };
        merge(&mut t, theme);
    }
    merge(&mut t, user);
    build(t)
}

pub fn font_bytes(name: &str) -> Result<Vec<u8>, String> {
    let home = dirs::home_dir().unwrap_or_default();
    let spots = [
        PathBuf::from(name),
        home.join("Library/Fonts").join(name),
        PathBuf::from("/Library/Fonts").join(name),
        PathBuf::from("/System/Library/Fonts").join(name),
    ];
    spots
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .ok_or_else(|| format!("font file {name:?} not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses_and_user_overrides_win() {
        let (cfg, pal) = defaults();
        assert_eq!(cfg.font_size, 14.0);
        assert_eq!(pal.bg, Rgb { r: 0x1e, g: 0x1e, b: 0x2e });

        let mut t: Table = DEFAULT.parse().unwrap();
        merge(&mut t, "font_size = 20.0\n[colors]\nbackground = \"#000000\"".parse().unwrap());
        let (cfg, pal) = build(t).unwrap();
        assert_eq!(cfg.font_size, 20.0);
        assert_eq!(pal.bg, Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(pal.fg, Rgb { r: 0xcd, g: 0xd6, b: 0xf4 }); // untouched sibling key survives
    }

    #[test]
    fn builtin_themes_build() {
        for (name, src) in THEMES {
            let mut t: Table = DEFAULT.parse().unwrap();
            merge(&mut t, src.parse().unwrap());
            assert!(build(t).is_ok(), "theme {name}");
        }
    }

    #[test]
    fn bad_color_is_an_error() {
        assert!(hex("#12345").is_err());
        assert!(hex("red").is_err());
    }
}
