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

impl RecentBook {
    pub fn progress_percent(&self) -> u32 {
        (self.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    }
}
