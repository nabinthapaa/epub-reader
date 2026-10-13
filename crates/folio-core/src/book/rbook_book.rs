//! The only file that knows about rbook.
//!
//! Everything EPUB-specific (spine order, manifest, NCX vs nav document,
//! percent-encoded hrefs) is resolved here, once, at open time, and handed on
//! as the plain structures in [`super::ports`].

use std::path::Path;

use rbook::Epub;

use super::path;
use super::ports::{Book, BookMeta, Target, TocItem};
use crate::error::{Error, Result};

pub struct RbookBook {
    epub: Epub,
    meta: BookMeta,
    /// Normalized archive path of every spine entry, in reading order.
    chapters: Vec<String>,
    toc: Vec<TocItem>,
    cover_path: Option<String>,
}

impl RbookBook {
    /// Open an EPUB file (or an unzipped EPUB directory).
    pub fn open(file: &Path) -> Result<Self> {
        let epub = Epub::open(file).map_err(|e| Error::Ebook(e.to_string()))?;

        let meta = {
            let metadata = epub.metadata();
            BookMeta {
                title: metadata
                    .title()
                    .map(|t| t.value().to_string())
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| {
                        file.file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Untitled".into())
                    }),
                authors: metadata.creators().map(|c| c.value().to_string()).collect(),
                language: metadata.languages().next().map(|l| l.value().to_string()),
            }
        };

        let chapters: Vec<String> = epub
            .spine()
            .iter()
            .filter_map(|entry| {
                let resource = entry.resource()?;
                let raw = resource.key().value()?;
                Some(path::normalize(&path::percent_decode(raw)))
            })
            .collect();

        if chapters.is_empty() {
            return Err(Error::Empty);
        }

        let toc = collect_toc(&epub, &chapters);

        let cover_path = find_cover(&epub);

        Ok(Self {
            epub,
            meta,
            chapters,
            toc,
            cover_path,
        })
    }

    /// Index of the spine entry stored at `path`, if any.
    fn chapter_at_path(&self, path: &str) -> Option<usize> {
        self.chapters.iter().position(|c| c == path)
    }
}

/// Locate the cover image.
///
/// EPUB 3 declares it with a manifest property, but plenty of EPUB 2 files only
/// hint at it through a `cover` id or a file named like one, so fall back to
/// looking for that before giving up.
fn find_cover(epub: &Epub) -> Option<String> {
    let normalize_entry = |raw: &str| path::normalize(&path::percent_decode(raw));

    if let Some(entry) = epub.manifest().cover_image()
        && let Some(raw) = entry.resource().key().value()
    {
        return Some(normalize_entry(raw));
    }

    let images: Vec<String> = epub
        .manifest()
        .images()
        .filter_map(|entry| entry.resource().key().value().map(normalize_entry))
        .collect();

    images
        .iter()
        .find(|path| path.to_ascii_lowercase().contains("cover"))
        .or_else(|| images.first())
        .cloned()
}

/// Flatten the book's table of contents, mapping each entry onto a spine index.
fn collect_toc(epub: &Epub, chapters: &[String]) -> Vec<TocItem> {
    let toc = epub.toc();
    let Some(root) = toc.contents() else {
        return Vec::new();
    };
    root.flatten()
        .map(|entry| {
            let target = entry
                .manifest_entry()
                .and_then(|manifest_entry| {
                    let resource = manifest_entry.resource();
                    let raw = resource.key().value()?.to_string();
                    Some(path::normalize(&path::percent_decode(&raw)))
                })
                .and_then(|file| {
                    let chapter = chapters.iter().position(|c| *c == file)?;
                    let anchor = entry
                        .href()
                        .and_then(|href| href.fragment().map(path::percent_decode));
                    Some(Target { chapter, anchor })
                });
            TocItem {
                label: entry.label().trim().to_string(),
                depth: entry.depth().saturating_sub(1),
                target,
            }
        })
        .collect()
}

impl Book for RbookBook {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_count(&self) -> usize {
        self.chapters.len()
    }

    fn chapter_path(&self, chapter: usize) -> Option<&str> {
        self.chapters.get(chapter).map(String::as_str)
    }

    fn chapter_xhtml(&self, chapter: usize) -> Result<String> {
        let path = self
            .chapters
            .get(chapter)
            .ok_or(Error::NoSuchChapter(chapter))?;
        // rbook resolves bare paths against the OPF directory; our normalized
        // paths are archive-absolute, so they go back with a leading slash.
        self.epub
            .read_resource_str(format!("/{path}").as_str())
            .map_err(|e| Error::Ebook(format!("{path}: {e}")))
    }

    fn toc(&self) -> &[TocItem] {
        &self.toc
    }

    fn resource(&self, path: &str) -> Result<Vec<u8>> {
        self.epub
            .read_resource_bytes(format!("/{path}").as_str())
            .map_err(|e| Error::Ebook(format!("{path}: {e}")))
    }

    fn cover(&self) -> Option<Vec<u8>> {
        let path = self.cover_path.as_deref()?;
        self.resource(path).ok()
    }

    fn resolve_link(&self, chapter: usize, href: &str) -> Option<Target> {
        if path::is_external(href) {
            return None;
        }
        let (raw_path, fragment) = path::split_fragment(href);
        let anchor = fragment.map(path::percent_decode);

        if raw_path.is_empty() {
            // A bare "#anchor" points inside the current chapter.
            return Some(Target { chapter, anchor });
        }

        let base = self.chapter_path(chapter)?;
        let resolved = path::resolve(base, &path::percent_decode(raw_path));
        let target_chapter = self.chapter_at_path(&resolved)?;
        Some(Target {
            chapter: target_chapter,
            anchor,
        })
    }
}
