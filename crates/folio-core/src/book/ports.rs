//! The port the rest of the app sees instead of an EPUB library.
//!
//! Only `rbook_book.rs` knows that rbook (or EPUB, or zip) exists. Everything
//! else depends on this trait, which keeps the container format swappable and
//! lets tests substitute an in-memory book.

use crate::error::Result;

/// Bibliographic details shown in the title bar and on the shelf.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub language: Option<String>,
}

impl BookMeta {
    /// "Author — Title" style line for the window title.
    pub fn display_line(&self) -> String {
        match self.authors.first() {
            Some(author) if !author.is_empty() => format!("{} — {}", self.title, author),
            _ => self.title.clone(),
        }
    }
}

/// A place in the book: a spine entry plus an optional fragment inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub chapter: usize,
    pub anchor: Option<String>,
}

/// One row of the table of contents, flattened with its nesting depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocItem {
    pub label: String,
    pub depth: usize,
    /// `None` when the entry is a grouping label with no destination.
    pub target: Option<Target>,
}

/// What the reader needs from a book, and nothing more.
pub trait Book: Send + Sync {
    fn meta(&self) -> &BookMeta;

    /// Number of readable chapters, in reading order.
    fn chapter_count(&self) -> usize;

    /// Archive path of a chapter, used to resolve relative hrefs.
    fn chapter_path(&self, chapter: usize) -> Option<&str>;

    /// The chapter's markup.
    fn chapter_xhtml(&self, chapter: usize) -> Result<String>;

    /// Size of a chapter's resource in bytes, if it can be had cheaply.
    ///
    /// Used only as a relative weight for progress, so markup overhead does
    /// not matter: what matters is that the number never changes while the
    /// book is open.
    fn chapter_bytes(&self, chapter: usize) -> Option<usize> {
        let _ = chapter;
        None
    }

    fn toc(&self) -> &[TocItem];

    /// Raw bytes of any resource, addressed by normalized archive path.
    fn resource(&self, path: &str) -> Result<Vec<u8>>;

    /// Cover image bytes, if the book declares one.
    fn cover(&self) -> Option<Vec<u8>>;

    /// Turn an href found in `chapter` into a place in this book.
    /// Returns `None` for external links and unknown destinations.
    fn resolve_link(&self, chapter: usize, href: &str) -> Option<Target>;
}
