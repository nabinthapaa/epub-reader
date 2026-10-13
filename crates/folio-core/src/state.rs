//! Reading position and persisted settings.

use serde::{Deserialize, Serialize};

use crate::layout::TypeSettings;
use crate::theme::ThemeChoice;

/// A place in a book, expressed so that it survives zoom, resize and font
/// changes: pages are derived, never stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Spine index.
    pub chapter: usize,
    /// Block index within the chapter.
    pub block: usize,
    /// Character offset into that block's text.
    #[serde(default)]
    pub char_offset: usize,
}

impl Position {
    pub fn chapter_start(chapter: usize) -> Self {
        Self {
            chapter,
            block: 0,
            char_offset: 0,
        }
    }
}

/// Everything the app remembers between runs, other than per-book positions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    #[serde(flatten)]
    pub type_settings: TypeSettings,
    /// Dim non-active paragraphs while in focus mode.
    pub spotlight: bool,
    /// Whether the contents panel is open.
    pub toc_open: bool,
    pub window_width: u32,
    pub window_height: u32,
}

impl Settings {
    /// Clamp anything that came off disk into a range the layout engine can
    /// actually use. cosmic-text asserts on a zero line height, and that
    /// assert runs on the render thread, where a panic freezes the window.
    pub fn sanitize(mut self) -> Self {
        let type_settings = &mut self.type_settings;
        if !type_settings.font_px.is_finite() {
            type_settings.font_px = TypeSettings::default().font_px;
        }
        if !type_settings.line_height.is_finite() {
            type_settings.line_height = TypeSettings::default().line_height;
        }
        if !type_settings.measure_ch.is_finite() {
            type_settings.measure_ch = TypeSettings::default().measure_ch;
        }
        if !type_settings.margin_px.is_finite() {
            type_settings.margin_px = TypeSettings::default().margin_px;
        }
        type_settings.font_px = type_settings.font_px.clamp(
            crate::layout::ZOOM_STEPS[0],
            crate::layout::ZOOM_STEPS[crate::layout::ZOOM_STEPS.len() - 1],
        );
        type_settings.line_height = type_settings.line_height.clamp(1.0, 2.5);
        type_settings.measure_ch = type_settings.measure_ch.clamp(40.0, 110.0);
        type_settings.margin_px = type_settings.margin_px.clamp(0.0, 200.0);
        self.window_width = self.window_width.clamp(400, 16_384);
        self.window_height = self.window_height.clamp(300, 16_384);
        self
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            type_settings: TypeSettings::default(),
            spotlight: false,
            toc_open: false,
            window_width: 1000,
            window_height: 760,
        }
    }
}

/// One entry on the recents shelf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentBook {
    pub path: String,
    pub title: String,
    pub author: String,
    pub position: Position,
    /// 0.0..=1.0, for the progress label on the tile.
    pub progress: f32,
    /// File name of the cached cover inside the covers directory.
    pub cover: Option<String>,
    /// Seconds since the Unix epoch.
    pub opened_at: u64,
}

impl Default for RecentBook {
    fn default() -> Self {
        Self {
            path: String::new(),
            title: String::new(),
            author: String::new(),
            position: Position::default(),
            progress: 0.0,
            cover: None,
            opened_at: 0,
        }
    }
}

impl RecentBook {
    pub fn progress_percent(&self) -> u32 {
        (self.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    }
}
