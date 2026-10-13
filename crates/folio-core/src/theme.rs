//! Theme tokens.
//!
//! Colors live here and nowhere else: the rasterizer reads them for glyphs and
//! rules, and the Slint layer receives the same struct for its chrome. Because
//! publisher CSS is discarded, nothing in a book can fight these values.

use serde::{Deserialize, Serialize};

/// Straight (non-premultiplied) 8-bit RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Scale the alpha channel, used by focus-mode dimming.
    pub fn fade(self, factor: f32) -> Self {
        let a = (self.a as f32 * factor.clamp(0.0, 1.0)).round() as u8;
        Self { a, ..self }
    }

    /// Mix towards `other` by `t` in 0..=1.
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }
}

/// The concrete palettes a reader can be in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreset {
    #[default]
    Light,
    Sepia,
    Dark,
}

impl ThemePreset {
    pub const ALL: [ThemePreset; 3] = [ThemePreset::Light, ThemePreset::Sepia, ThemePreset::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemePreset::Light => "Light",
            ThemePreset::Sepia => "Sepia",
            ThemePreset::Dark => "Dark",
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, ThemePreset::Dark)
    }
}

/// What the user asked for, which may be "whatever the desktop is doing".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "mode", content = "preset")]
pub enum ThemeChoice {
    /// Follow the system color scheme, mapping dark to [`ThemePreset::Dark`].
    #[default]
    System,
    Fixed(ThemePreset),
}

impl ThemeChoice {
    /// Resolve to a preset given what the platform reports.
    pub fn resolve(self, system_is_dark: bool) -> ThemePreset {
        match self {
            ThemeChoice::System if system_is_dark => ThemePreset::Dark,
            ThemeChoice::System => ThemePreset::Light,
            ThemeChoice::Fixed(preset) => preset,
        }
    }
}

/// Every color the app draws with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub preset: ThemePreset,
    /// Page background behind the text.
    pub page: Color,
    /// Chrome background (toolbar, panels).
    pub surface: Color,
    pub text: Color,
    /// De-emphasized text: captions, page numbers, list markers.
    pub muted: Color,
    pub accent: Color,
    pub link: Color,
    pub selection: Color,
    pub border: Color,
    /// Alpha applied to non-active paragraphs when the spotlight is on.
    pub dim_factor: f32,
}

impl Theme {
    pub fn new(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::Light => Self {
                preset,
                page: Color::rgb(0xFC, 0xFC, 0xFA),
                surface: Color::rgb(0xF1, 0xF0, 0xEC),
                text: Color::rgb(0x1E, 0x1E, 0x1C),
                muted: Color::rgb(0x6B, 0x6B, 0x66),
                accent: Color::rgb(0x33, 0x5C, 0x81),
                link: Color::rgb(0x1F, 0x56, 0x8C),
                selection: Color::rgba(0x33, 0x5C, 0x81, 0x40),
                border: Color::rgb(0xD8, 0xD6, 0xD0),
                dim_factor: 0.35,
            },
            ThemePreset::Sepia => Self {
                preset,
                page: Color::rgb(0xF6, 0xEC, 0xD9),
                surface: Color::rgb(0xEB, 0xDF, 0xC6),
                text: Color::rgb(0x3B, 0x2F, 0x21),
                muted: Color::rgb(0x7A, 0x68, 0x51),
                accent: Color::rgb(0x8A, 0x5A, 0x2B),
                link: Color::rgb(0x8A, 0x4F, 0x1D),
                selection: Color::rgba(0x8A, 0x5A, 0x2B, 0x3A),
                border: Color::rgb(0xD9, 0xC9, 0xAA),
                dim_factor: 0.35,
            },
            ThemePreset::Dark => Self {
                preset,
                // Prose at ~9.8:1 rather than ~12.9:1: bright enough to read,
                // dim enough not to halate over an hour. Surface and border are
                // lifted so the chrome separates from the page by more than a
                // hairline.
                page: Color::rgb(0x1A, 0x1C, 0x1F),
                surface: Color::rgb(0x24, 0x26, 0x2A),
                text: Color::rgb(0xCD, 0xCA, 0xC3),
                muted: Color::rgb(0x8C, 0x8C, 0x88),
                accent: Color::rgb(0x8A, 0xB4, 0xDE),
                link: Color::rgb(0x8A, 0xB4, 0xDE),
                selection: Color::rgba(0x8A, 0xB4, 0xDE, 0x40),
                border: Color::rgb(0x38, 0x3C, 0x42),
                dim_factor: 0.34,
            },
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemePreset::default())
    }
}
