//! Folio's reading core: EPUB in, laid-out pages out.
//!
//! This crate has no UI dependency. It parses books, discards publisher CSS,
//! shapes and paginates text, rasterizes pages into plain RGBA buffers, and
//! owns the reading cursor. The Slint binary is a projection of [`reader::View`].

pub mod book;
pub mod content;
pub mod error;
pub mod layout;
pub mod reader;
pub mod state;
pub mod store;
pub mod theme;

pub use book::{Book, BookMeta, TocItem};
pub use content::{Block, BlockKind, Chapter};
pub use error::{Error, Result};
pub use layout::{Canvas, TypeSettings, ZOOM_STEPS};
pub use reader::{Command, Reader, SearchHit, View};
pub use state::{Position, RecentBook, Settings};
pub use store::Store;
pub use theme::{Theme, ThemeChoice, ThemePreset};
