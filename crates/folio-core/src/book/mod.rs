//! Getting a book's structure and bytes, without the rest of the app knowing
//! what a container format is.

pub mod path;
pub mod ports;
pub mod rbook_book;

pub use ports::{Book, BookMeta, Target, TocItem};
pub use rbook_book::RbookBook;
