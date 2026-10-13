//! Error type shared by the whole core.

/// Result alias used throughout `folio-core`.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The ebook could not be opened or a resource inside it could not be read.
    #[error("ebook: {0}")]
    Ebook(String),

    /// The book opened but contains nothing readable.
    #[error("the book has no readable chapters")]
    Empty,

    /// A chapter index outside the spine was requested.
    #[error("no chapter at index {0}")]
    NoSuchChapter(usize),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("settings file: {0}")]
    Json(#[from] serde_json::Error),
}
