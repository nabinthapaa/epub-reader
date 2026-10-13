//! Test doubles: an in-memory book, and a real EPUB built on the fly.
//!
//! Fixtures are generated rather than committed, so the suite carries no binary
//! blobs and every test states the structure it depends on.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Cursor, Write};

use folio_core::book::path as href_path;
use folio_core::book::ports::{Book, BookMeta, Target, TocItem};
use folio_core::error::{Error, Result};

/// A book held entirely in memory, bypassing zip and OPF parsing.
pub struct FakeBook {
    meta: BookMeta,
    chapters: Vec<(String, String)>,
    toc: Vec<TocItem>,
    resources: HashMap<String, Vec<u8>>,
}

impl FakeBook {
    /// Build from `(path, xhtml)` pairs in spine order.
    pub fn new(chapters: Vec<(&str, &str)>) -> Self {
        let chapters: Vec<(String, String)> = chapters
            .into_iter()
            .map(|(path, xhtml)| (path.to_string(), xhtml.to_string()))
            .collect();
        let toc = chapters
            .iter()
            .enumerate()
            .map(|(index, _)| TocItem {
                label: format!("Chapter {}", index + 1),
                depth: 0,
                target: Some(Target {
                    chapter: index,
                    anchor: None,
                }),
            })
            .collect();
        Self {
            meta: BookMeta {
                title: "Test Book".into(),
                authors: vec!["A. Tester".into()],
                language: Some("en".into()),
            },
            chapters,
            toc,
            resources: HashMap::new(),
        }
    }

    pub fn with_resource(mut self, path: &str, bytes: Vec<u8>) -> Self {
        self.resources.insert(path.to_string(), bytes);
        self
    }

    pub fn with_toc(mut self, toc: Vec<TocItem>) -> Self {
        self.toc = toc;
        self
    }
}

impl Book for FakeBook {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_count(&self) -> usize {
        self.chapters.len()
    }

    fn chapter_path(&self, chapter: usize) -> Option<&str> {
        self.chapters.get(chapter).map(|(path, _)| path.as_str())
    }

    fn chapter_xhtml(&self, chapter: usize) -> Result<String> {
        self.chapters
            .get(chapter)
            .map(|(_, xhtml)| xhtml.clone())
            .ok_or(Error::NoSuchChapter(chapter))
    }

    fn toc(&self) -> &[TocItem] {
        &self.toc
    }

    fn resource(&self, path: &str) -> Result<Vec<u8>> {
        self.resources
            .get(path)
            .cloned()
            .ok_or_else(|| Error::Ebook(format!("missing {path}")))
    }

    fn cover(&self) -> Option<Vec<u8>> {
        None
    }

    fn resolve_link(&self, chapter: usize, href: &str) -> Option<Target> {
        if href_path::is_external(href) {
            return None;
        }
        let (raw, fragment) = href_path::split_fragment(href);
        let anchor = fragment.map(href_path::percent_decode);
        if raw.is_empty() {
            return Some(Target { chapter, anchor });
        }
        let base = self.chapter_path(chapter)?;
        let resolved = href_path::resolve(base, &href_path::percent_decode(raw));
        let target = self
            .chapters
            .iter()
            .position(|(path, _)| *path == resolved)?;
        Some(Target {
            chapter: target,
            anchor,
        })
    }
}

/// A minimal but valid EPUB 3 archive, for exercising the rbook adapter.
pub fn build_epub(chapters: &[(&str, &str)]) -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        // The mimetype entry must be first and uncompressed.
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflated = zip::write::SimpleFileOptions::default();
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
        )
        .unwrap();

        let manifest: String = chapters
            .iter()
            .enumerate()
            .map(|(index, (name, _))| {
                format!(r#"<item id="c{index}" href="{name}" media-type="application/xhtml+xml"/>"#)
            })
            .collect();
        let spine: String = (0..chapters.len())
            .map(|index| format!(r#"<itemref idref="c{index}"/>"#))
            .collect();
        let nav_items: String = chapters
            .iter()
            .enumerate()
            .map(|(index, (name, _))| {
                format!(r#"<li><a href="{name}">Chapter {}</a></li>"#, index + 1)
            })
            .collect();

        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">urn:uuid:folio-test</dc:identifier>
    <dc:title>Test Book</dc:title>
    <dc:creator>A. Tester</dc:creator>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    {manifest}
  </manifest>
  <spine>{spine}</spine>
</package>"#
            )
            .as_bytes(),
        )
        .unwrap();

        zip.start_file("OEBPS/nav.xhtml", deflated).unwrap();
        zip.write_all(
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Contents</title></head>
<body><nav epub:type="toc"><ol>{nav_items}</ol></nav></body></html>"#
            )
            .as_bytes(),
        )
        .unwrap();

        for (name, body) in chapters {
            zip.start_file(format!("OEBPS/{name}"), deflated).unwrap();
            zip.write_all(
                format!(
                    r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>{name}</title></head>
<body>{body}</body></html>"#
                )
                .as_bytes(),
            )
            .unwrap();
        }
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

/// Write a generated EPUB to a temporary file and return its path.
pub fn write_epub(chapters: &[(&str, &str)], name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("folio-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, build_epub(chapters)).unwrap();
    path
}

/// Long filler text, for making chapters that span several pages.
pub fn paragraphs(count: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "<p>Paragraph {index}. The quick brown fox jumps over the lazy dog, \
                 and keeps on jumping until the line has to wrap at least once more.</p>"
            )
        })
        .collect()
}
