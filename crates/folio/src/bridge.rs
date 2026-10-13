//! Conversions between core types and the generated Slint types.

use std::path::Path;

use folio_core::book::TocItem;
use folio_core::reader::SearchHit;
use folio_core::state::RecentBook;
use folio_core::theme::{Color, Theme};
use slint::{ModelRc, SharedString, VecModel};

use crate::{SearchRow, ShelfRow, TocRow, Tokens};

pub fn color(color: Color) -> slint::Color {
    slint::Color::from_argb_u8(color.a, color.r, color.g, color.b)
}

pub fn tokens(theme: &Theme) -> Tokens {
    Tokens {
        page: color(theme.page),
        surface: color(theme.surface),
        text: color(theme.text),
        muted: color(theme.muted),
        accent: color(theme.accent),
        link: color(theme.link),
        border: color(theme.border),
    }
}

/// Build the table of contents, highlighting the row the reader is inside.
pub fn toc_rows(items: &[TocItem], current: Option<usize>) -> ModelRc<TocRow> {
    let rows: Vec<TocRow> = items
        .iter()
        .enumerate()
        .map(|(index, item)| TocRow {
            label: SharedString::from(item.label.as_str()),
            depth: item.depth.min(4) as i32,
            index: index as i32,
            current: Some(index) == current,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

pub fn search_rows(hits: &[SearchHit]) -> ModelRc<SearchRow> {
    let rows: Vec<SearchRow> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| SearchRow {
            label: SharedString::from(hit.chapter_label.as_str()),
            snippet: SharedString::from(hit.snippet.as_str()),
            index: index as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

pub fn shelf_rows(recents: &[RecentBook], covers_dir: &Path) -> ModelRc<ShelfRow> {
    let rows: Vec<ShelfRow> = recents
        .iter()
        .enumerate()
        .map(|(index, book)| ShelfRow {
            title: SharedString::from(book.title.as_str()),
            // "Unknown" is what some EPUBs literally put in the creator field;
            // showing nothing is more honest than showing that.
            author: SharedString::from(if book.author.eq_ignore_ascii_case("unknown") {
                ""
            } else {
                book.author.as_str()
            }),
            progress: book.progress_percent() as i32,
            cover: book
                .cover
                .as_ref()
                .and_then(|name| slint::Image::load_from_path(&covers_dir.join(name)).ok())
                .unwrap_or_default(),
            index: index as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// Wrap raw RGBA pixels as a Slint image.
pub fn image_from_rgba(rgba: &[u8], width: u32, height: u32) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(width, height);
    buffer.make_mut_bytes().copy_from_slice(rgba);
    slint::Image::from_rgba8(buffer)
}
