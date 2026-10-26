// Professional clean terminal backdrop canvas
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::fs;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub const BACKDROP_WIDTH: usize = 160;
#[allow(dead_code)]
pub const BACKDROP_HEIGHT: usize = 72;
#[allow(dead_code)]
pub static BACKDROP_IMAGE_BYTES: &[u8] = b"";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackdropMode {
    Subtle,
    Vivid,
    Off,
}

impl BackdropMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Subtle => "subtle",
            Self::Vivid => "vivid",
            Self::Off => "off",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Subtle => Self::Vivid,
            Self::Vivid => Self::Off,
            Self::Off => Self::Subtle,
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "subtle" | "on" | "default" => Some(Self::Subtle),
            "vivid" | "bright" | "full" => Some(Self::Vivid),
            "off" | "none" | "disable" | "disabled" => Some(Self::Off),
            _ => None,
        }
    }
}

pub fn ensure_backdrop_asset(data_dir: &Path) -> Option<PathBuf> {
    let assets_dir = data_dir.join("assets");
    fs::create_dir_all(&assets_dir).ok()?;
    let path = assets_dir.join("terminal-background.jpg");
    if !path.is_file() {
        fs::write(&path, b"").ok()?;
    }
    Some(path)
}

pub fn apply_backdrop(buf: &mut Buffer, area: Rect, mode: BackdropMode, is_light: bool) {
    if mode == BackdropMode::Off || area.width == 0 || area.height == 0 {
        return;
    }

    // Professional clean palette:
    // Dark mode: clean deep obsidian slate with subtle violet hue
    // Light mode: clean crisp parchment slate
    let bg_color = match (mode, is_light) {
        (BackdropMode::Subtle, false) => Color::Rgb(12, 14, 20),
        (BackdropMode::Vivid, false) => Color::Rgb(18, 15, 29),
        (BackdropMode::Subtle, true) => Color::Rgb(248, 249, 250),
        (BackdropMode::Vivid, true) => Color::Rgb(241, 243, 245),
        (BackdropMode::Off, _) => return,
    };

    for row in 0..area.height {
        let y = area.y + row;
        for col in 0..area.width {
            let x = area.x + col;
            let cell = &mut buf[(x, y)];
            if cell.bg == Color::Reset {
                cell.set_bg(bg_color);
            }
        }
    }
}
