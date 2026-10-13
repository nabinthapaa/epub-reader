//! Typographic settings: the knobs zoom and the settings panel turn.

use serde::{Deserialize, Serialize};

/// The discrete zoom ladder, in pixels of base font size.
pub const ZOOM_STEPS: [f32; 10] = [12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 34.0, 40.0, 48.0];

/// Index into [`ZOOM_STEPS`] used when nothing is stored yet.
pub const DEFAULT_ZOOM_INDEX: usize = 3;

/// Paragraph alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    /// Ragged right: even word spacing, no hyphenation needed.
    #[default]
    Left,
    Justified,
}

/// Everything that affects where a glyph lands.
///
/// `#[serde(default)]` matters here: `Settings` flattens this struct, and a
/// container default on the outer type does not cover a flattened field.
/// Without it, adding one field would make every settings file written by an
/// earlier version fail to parse, and the reader would silently reset the
/// theme, the window size and everything else alongside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypeSettings {
    /// Base font size in px; headings scale off this.
    pub font_px: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Preferred body family; the first installed family in the fallback chain
    /// is used when this is `None`.
    pub family: Option<String>,
    /// Preferred monospace family for `code`.
    pub mono_family: Option<String>,
    pub alignment: Alignment,
    /// Hard cap on line length, in characters.
    pub measure_ch: f32,
    /// Minimum padding between the page edge and the text column, in px.
    pub margin_px: f32,
}

impl Default for TypeSettings {
    fn default() -> Self {
        Self {
            font_px: ZOOM_STEPS[DEFAULT_ZOOM_INDEX],
            line_height: 1.6,
            family: None,
            mono_family: None,
            alignment: Alignment::Left,
            measure_ch: 70.0,
            margin_px: 48.0,
        }
    }
}

impl TypeSettings {
    /// Width of the text column inside a viewport `viewport_w` px wide.
    ///
    /// The column is capped at `measure_ch` characters (estimated at 0.5em per
    /// character, the usual average for a text face) so that widening the
    /// window adds margin rather than unreadably long lines.
    pub fn column_width(&self, viewport_w: f32, narrowed: bool) -> f32 {
        let measure = if narrowed {
            self.measure_ch * 0.88
        } else {
            self.measure_ch
        };
        let cap = measure * self.font_px * 0.5;
        let available = (viewport_w - 2.0 * self.margin_px).max(self.font_px * 8.0);
        available.min(cap)
    }

    /// Nearest zoom step at or below the current size.
    pub fn zoom_index(&self) -> usize {
        ZOOM_STEPS
            .iter()
            .rposition(|step| *step <= self.font_px + f32::EPSILON)
            .unwrap_or(DEFAULT_ZOOM_INDEX)
    }

    /// Step the zoom ladder by `delta` stops, clamped at both ends.
    /// Returns true when the size actually changed.
    pub fn zoom_by(&mut self, delta: i32) -> bool {
        let current = self.zoom_index() as i32;
        let next = (current + delta).clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize;
        let size = ZOOM_STEPS[next];
        if (size - self.font_px).abs() < f32::EPSILON {
            return false;
        }
        self.font_px = size;
        true
    }

    /// Back to the default reading size.
    pub fn zoom_reset(&mut self) -> bool {
        let size = ZOOM_STEPS[DEFAULT_ZOOM_INDEX];
        if (size - self.font_px).abs() < f32::EPSILON {
            return false;
        }
        self.font_px = size;
        true
    }

    /// Key identifying everything that invalidates a laid-out chapter.
    pub fn layout_key(&self) -> String {
        format!(
            "{:.2}/{:.2}/{:?}/{:?}/{:?}/{:.1}",
            self.font_px,
            self.line_height,
            self.family,
            self.mono_family,
            self.alignment,
            self.measure_ch
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_walks_the_ladder_and_clamps() {
        let mut t = TypeSettings::default();
        assert_eq!(t.font_px, 18.0);
        assert!(t.zoom_by(1));
        assert_eq!(t.font_px, 20.0);
        assert!(t.zoom_by(-2));
        assert_eq!(t.font_px, 16.0);
        for _ in 0..20 {
            t.zoom_by(-1);
        }
        assert_eq!(t.font_px, ZOOM_STEPS[0]);
        assert!(!t.zoom_by(-1));
        for _ in 0..20 {
            t.zoom_by(1);
        }
        assert_eq!(t.font_px, ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        assert!(t.zoom_reset());
        assert_eq!(t.font_px, ZOOM_STEPS[DEFAULT_ZOOM_INDEX]);
    }

    #[test]
    fn column_is_capped_by_the_measure() {
        let t = TypeSettings::default();
        // A very wide window does not produce a very long line.
        let wide = t.column_width(4000.0, false);
        assert!((wide - 70.0 * 18.0 * 0.5).abs() < 0.01);
        // A narrow window is limited by the available space instead.
        assert!(t.column_width(400.0, false) < wide);
        // Focus mode narrows the measure.
        assert!(t.column_width(4000.0, true) < wide);
    }
}
