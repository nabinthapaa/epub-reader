//! Publisher markup in, structure out.

pub mod blocks;
pub mod html;

pub use blocks::{Block, BlockKind, Chapter, Emphasis, Script, Span};
pub use html::parse_chapter;
