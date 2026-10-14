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
                // 13.2:1 rather than 16.2:1. Near-black on near-white is the
                // glare case on light that the dark palette was tuned away
                // from; this is the zone the dedicated readers sit in.
                text: Color::rgb(0x2E, 0x2E, 0x2B),
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
                // Chrome text sits on `surface`, and an active control's
                // label sits on `accent` at 0.18 over `surface`, which is
                // tighter still. The old 0x7A6851 and 0x8A5A2B managed 4.05
                // and 3.65 there; these clear 4.5.
                muted: Color::rgb(0x72, 0x60, 0x49),
                accent: Color::rgb(0x71, 0x4A, 0x23),
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
                // 4.61:1 on `surface` rather than 4.49.
                muted: Color::rgb(0x8E, 0x8E, 0x8A),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.1 relative luminance.
    fn luminance(color: Color) -> f64 {
        let channel = |value: u8| {
            let value = value as f64 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Composite `over` onto an opaque background, as the renderer does.
    fn over(top: Color, bottom: Color, alpha: f64) -> Color {
        let mix = |a: u8, b: u8| (a as f64 * alpha + b as f64 * (1.0 - alpha)).round() as u8;
        Color::rgb(
            mix(top.r, bottom.r),
            mix(top.g, bottom.g),
            mix(top.b, bottom.b),
        )
    }

    /// Every palette has to clear AA for small text, and the tight pair is the
    /// chrome one: `muted` on `surface`, not on `page`. Sepia used to miss it
    /// at 4.05:1, which is most of the status bar and every panel.
    #[test]
    fn chrome_text_is_legible_in_every_theme() {
        for preset in [ThemePreset::Light, ThemePreset::Sepia, ThemePreset::Dark] {
            let theme = Theme::new(preset);
            for (name, color) in [
                ("text", theme.text),
                ("muted", theme.muted),
                ("accent", theme.accent),
                ("link", theme.link),
            ] {
                for (surface_name, background) in [("page", theme.page), ("surface", theme.surface)]
                {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= 4.5,
                        "{preset:?}: {name} on {surface_name} is {ratio:.2}:1, under AA",
                    );
                }
            }
        }
    }

    /// An active toolbar chip and the current row of the contents both draw
    /// the accent on a wash of itself, which is tighter than the accent on
    /// bare surface and is the state a reader most needs to pick out.
    #[test]
    fn an_active_control_reads_against_its_own_tint() {
        // ToolButton uses 0.18, the contents row 0.16.
        for alpha in [0.18, 0.16] {
            for preset in [ThemePreset::Light, ThemePreset::Sepia, ThemePreset::Dark] {
                let theme = Theme::new(preset);
                let tint = over(theme.accent, theme.surface, alpha);
                let ratio = contrast(theme.accent, tint);
                assert!(
                    ratio >= 4.5,
                    "{preset:?}: accent on its own {alpha} tint is {ratio:.2}:1, under AA",
                );
            }
        }
    }

    /// Prose is read for an hour at a time, so it wants more than the minimum.
    #[test]
    fn body_text_is_comfortable_rather_than_merely_legal() {
        for preset in [ThemePreset::Light, ThemePreset::Sepia, ThemePreset::Dark] {
            let theme = Theme::new(preset);
            let ratio = contrast(theme.text, theme.page);
            assert!(
                (7.0..=14.0).contains(&ratio),
                "{preset:?}: prose at {ratio:.2}:1 is outside the comfortable band \
                 (under 7 is thin, over 14 halates on a dark page)",
            );
        }
    }
}
