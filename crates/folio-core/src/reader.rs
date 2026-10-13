//! The reading session: the single place that knows where the reader is.
//!
//! The UI sends [`Command`]s and gets a [`View`] back. Keeping the cursor,
//! caches and anchoring here means the whole reading model can be tested
//! headlessly, and the Slint layer stays a projection of this state.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use cosmic_text::SwashCache;
use lru::LruCache;

use crate::book::{Book, BookMeta, RbookBook, Target, TocItem, path as href_path};
use crate::content::{Chapter, parse_chapter};
use crate::error::{Error, Result};
use crate::layout::engine::ImageInfo;
use crate::layout::{
    Canvas, ChapterLayout, DecodedImage, FontStack, Layouter, Pagination, TypeSettings, paginate,
    raster::{LinkRegion, PageFrame, render_page},
};
use crate::state::{Position, Settings};
use crate::theme::{Theme, ThemePreset};

/// How many laid-out chapters to keep (current plus one each side).
const LAYOUT_CACHE: usize = 3;
/// How many parsed chapters to keep.
const CHAPTER_CACHE: usize = 6;
/// Decoded images held for the open book before the map is recycled.
const IMAGE_CACHE: usize = 48;

/// What the UI can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Re-render at the current position (after a theme change, for instance).
    Refresh,
    Resize {
        width: u32,
        height: u32,
    },
    /// Turn pages, crossing chapter boundaries.
    Page(i32),
    /// Jump whole chapters.
    Chapter(i32),
    GoTo(Position),
    /// Jump to a table-of-contents entry, honoring its fragment.
    GoToToc(usize),
    /// Follow an internal link found by hit-testing.
    FollowLink(String),
    /// Return to where the last link was followed from.
    Back,
    /// Step the zoom ladder.
    Zoom(i32),
    ZoomReset,
    SetTypeSettings(TypeSettings),
    SetTheme(Theme),
    SetFocus(bool),
    SetSpotlight(bool),
    /// Move the focus-mode spotlight by whole paragraphs.
    MoveSpotlight(i32),
}

/// One search result.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub position: Position,
    pub chapter_label: String,
    /// The match with a little context either side.
    pub snippet: String,
}

/// Everything the UI needs to draw one frame.
pub struct View {
    pub canvas: Canvas,
    pub links: Vec<LinkRegion>,
    pub chapter: usize,
    pub chapter_label: String,
    /// Row of the table of contents the reader is inside, if any.
    pub toc_index: Option<usize>,
    pub page: usize,
    pub page_count: usize,
    pub position: Position,
    /// Fraction of the book read, 0.0..=1.0.
    pub progress: f32,
    pub focus: bool,
    pub spotlight: bool,
    pub can_go_back: bool,
    pub font_px: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Cursor {
    chapter: usize,
    page: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LayoutKey {
    chapter: usize,
    width: u32,
    height: u32,
    narrowed: bool,
    type_key: String,
}

struct LayoutEntry {
    layout: ChapterLayout,
    pagination: Pagination,
}

pub struct Reader {
    book: Box<dyn Book>,
    book_path: String,
    layouter: Layouter,
    swash: SwashCache,
    stack: FontStack,
    settings: TypeSettings,
    theme: Theme,
    viewport: (u32, u32),
    focus: bool,
    spotlight: bool,
    cursor: Cursor,
    /// Index into the current layout's blocks.
    spotlight_block: usize,
    history: Vec<Position>,
    chapters: LruCache<usize, Arc<Chapter>>,
    layouts: LruCache<LayoutKey, LayoutEntry>,
    images: HashMap<String, DecodedImage>,
    /// Lazily filled plain text per chapter, for search.
    chapter_text: Vec<Option<Arc<str>>>,
}

impl Reader {
    /// Open a book and position the cursor at `start` (or its beginning).
    pub fn open(
        file: &Path,
        settings: &Settings,
        theme_preset: ThemePreset,
        viewport: (u32, u32),
        start: Option<Position>,
    ) -> Result<Self> {
        let book = RbookBook::open(file)?;
        Self::with_book(
            Box::new(book),
            file.to_string_lossy().into_owned(),
            settings,
            theme_preset,
            viewport,
            start,
        )
    }

    /// Same, for an already-constructed book (used by tests).
    pub fn with_book(
        book: Box<dyn Book>,
        book_path: String,
        settings: &Settings,
        theme_preset: ThemePreset,
        viewport: (u32, u32),
        start: Option<Position>,
    ) -> Result<Self> {
        if book.chapter_count() == 0 {
            return Err(Error::Empty);
        }
        let layouter = Layouter::new();
        let stack = FontStack::resolve(
            &layouter.fonts,
            settings.type_settings.family.as_deref(),
            settings.type_settings.mono_family.as_deref(),
        );
        let chapter_count = book.chapter_count();
        let start = start.unwrap_or_default();
        let mut reader = Self {
            book,
            book_path,
            layouter,
            swash: SwashCache::new(),
            stack,
            settings: settings.type_settings.clone(),
            theme: Theme::new(theme_preset),
            viewport,
            focus: false,
            spotlight: settings.spotlight,
            cursor: Cursor {
                chapter: start.chapter.min(chapter_count - 1),
                page: 0,
            },
            spotlight_block: 0,
            history: Vec::new(),
            chapters: LruCache::new(NonZeroUsize::new(CHAPTER_CACHE).unwrap()),
            layouts: LruCache::new(NonZeroUsize::new(LAYOUT_CACHE).unwrap()),
            images: HashMap::new(),
            chapter_text: vec![None; chapter_count],
        };
        reader.go_to(start)?;
        Ok(reader)
    }

    pub fn meta(&self) -> &BookMeta {
        self.book.meta()
    }

    pub fn toc(&self) -> &[TocItem] {
        self.book.toc()
    }

    pub fn book_path(&self) -> &str {
        &self.book_path
    }

    pub fn chapter_count(&self) -> usize {
        self.book.chapter_count()
    }

    pub fn type_settings(&self) -> &TypeSettings {
        &self.settings
    }

    pub fn cover(&self) -> Option<Vec<u8>> {
        self.book.cover()
    }

    /// The body family text is actually shaped with, after the preference
    /// chain has been resolved against what is installed.
    pub fn body_family(&self) -> String {
        self.stack.body.clone()
    }

    /// Available body font families, for the settings panel.
    pub fn font_families(&self) -> Vec<String> {
        crate::layout::installed_families(&self.layouter.fonts)
    }

    /// Apply a command and render the resulting frame.
    pub fn apply(&mut self, command: Command) -> Result<View> {
        match command {
            Command::Refresh => {}
            Command::Resize { width, height } => {
                if self.viewport == (width, height) {
                    return self.render();
                }
                let anchor = self.position()?;
                self.viewport = (width, height);
                self.go_to(anchor)?;
            }
            Command::Page(delta) => self.turn_pages(delta)?,
            Command::Chapter(delta) => {
                let target = (self.cursor.chapter as i64 + delta as i64)
                    .clamp(0, self.chapter_count() as i64 - 1)
                    as usize;
                self.go_to(Position::chapter_start(target))?;
            }
            Command::GoTo(position) => self.go_to(position)?,
            Command::GoToToc(index) => {
                if let Some(target) = self
                    .book
                    .toc()
                    .get(index)
                    .and_then(|item| item.target.clone())
                {
                    let from = self.position()?;
                    let position =
                        self.position_for_anchor(target.chapter, target.anchor.as_deref())?;
                    self.history.push(from);
                    self.go_to(position)?;
                }
            }
            Command::FollowLink(href) => {
                if let Some(Target { chapter, anchor }) =
                    self.book.resolve_link(self.cursor.chapter, &href)
                {
                    let from = self.position()?;
                    let target = self.position_for_anchor(chapter, anchor.as_deref())?;
                    self.history.push(from);
                    self.go_to(target)?;
                }
            }
            Command::Back => {
                if let Some(previous) = self.history.pop() {
                    self.go_to(previous)?;
                }
            }
            Command::Zoom(delta) => {
                let mut settings = self.settings.clone();
                if settings.zoom_by(delta) {
                    self.replace_settings(settings)?;
                }
            }
            Command::ZoomReset => {
                let mut settings = self.settings.clone();
                if settings.zoom_reset() {
                    self.replace_settings(settings)?;
                }
            }
            Command::SetTypeSettings(settings) => self.replace_settings(settings)?,
            Command::SetTheme(theme) => self.theme = theme,
            Command::SetFocus(focus) => {
                if self.focus != focus {
                    let anchor = self.position()?;
                    self.focus = focus;
                    self.go_to(anchor)?;
                }
            }
            Command::SetSpotlight(spotlight) => self.spotlight = spotlight,
            Command::MoveSpotlight(delta) => self.move_spotlight(delta)?,
        }
        self.render()
    }

    /// Current position, derived from the first line on the page.
    pub fn position(&mut self) -> Result<Position> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let page = self.cursor.page;
        let Some(entry) = self.layouts.peek(&key) else {
            return Ok(Position::chapter_start(self.cursor.chapter));
        };
        let Some((block_index, line)) = entry
            .pagination
            .pages
            .get(page)
            .and_then(|page| page.first_line())
        else {
            return Ok(Position::chapter_start(self.cursor.chapter));
        };
        let block = entry
            .layout
            .blocks
            .get(block_index)
            .map(|b| b.block)
            .unwrap_or(0);
        let char_offset = paginate::char_for_line(&entry.layout, block_index, line);
        Ok(Position {
            chapter: self.cursor.chapter,
            block,
            char_offset,
        })
    }

    /// Fraction of the book behind the current page.
    pub fn progress(&mut self) -> f32 {
        let chapters = self.chapter_count().max(1) as f32;
        let page_count = self.page_count().max(1) as f32;
        let within = (self.cursor.page as f32 + 1.0) / page_count;
        ((self.cursor.chapter as f32 + within) / chapters).clamp(0.0, 1.0)
    }

    /// Search the whole book, cheapest-first: already-parsed chapters are
    /// scanned from cache, the rest are parsed on demand.
    pub fn search(&mut self, query: &str, limit: usize) -> Vec<SearchHit> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for chapter in 0..self.chapter_count() {
            if hits.len() >= limit {
                break;
            }
            let Ok(parsed) = self.chapter(chapter) else {
                continue;
            };
            for (block_index, block) in parsed.blocks.iter().enumerate() {
                if hits.len() >= limit {
                    break;
                }
                let text = block.text();
                let haystack = text.to_lowercase();
                if let Some(byte_index) = haystack.find(&needle) {
                    hits.push(SearchHit {
                        position: Position {
                            chapter,
                            block: block_index,
                            char_offset: text[..byte_index].chars().count(),
                        },
                        chapter_label: self
                            .book
                            .toc()
                            .iter()
                            .rfind(|item| {
                                item.target.as_ref().is_some_and(|t| t.chapter <= chapter)
                                    && !item.label.trim().is_empty()
                            })
                            .map(|item| item.label.clone())
                            .unwrap_or_else(|| format!("Chapter {}", chapter + 1)),
                        snippet: snippet_around(&text, byte_index, needle.len()),
                    });
                }
            }
        }
        hits
    }

    /// The table-of-contents row the current page sits inside.
    ///
    /// Books that put many chapters in one spine file (most Gutenberg EPUBs do)
    /// need the entry's fragment to be resolved against the laid-out blocks,
    /// not just its spine index, or every page claims to be the last chapter in
    /// the file.
    pub fn active_toc(&mut self) -> Option<usize> {
        let chapter = self.cursor.chapter;
        let current_block = self.first_block_on_page();
        let key = self.ensure_layout(chapter).ok()?;
        let entry = self.layouts.peek(&key)?;

        let mut best: Option<(usize, usize)> = None;
        let mut fallback: Option<usize> = None;
        for (index, item) in self.book.toc().iter().enumerate() {
            let Some(target) = &item.target else { continue };
            if item.label.trim().is_empty() {
                continue;
            }
            if target.chapter < chapter {
                fallback = Some(index);
                continue;
            }
            if target.chapter > chapter {
                continue;
            }
            let block = match target.anchor.as_deref() {
                Some(anchor) => entry.layout.anchors.get(anchor).copied().unwrap_or(0),
                None => 0,
            };
            if block <= current_block && best.is_none_or(|(_, b)| block >= b) {
                best = Some((index, block));
            }
        }
        best.map(|(index, _)| index).or(fallback)
    }

    /// Human-readable label for the current page.
    pub fn current_label(&mut self) -> String {
        let chapter = self.cursor.chapter;
        self.active_toc()
            .and_then(|index| self.book.toc().get(index))
            .map(|item| item.label.clone())
            .unwrap_or_else(|| format!("Chapter {}", chapter + 1))
    }

    // --- internals ---------------------------------------------------------

    fn page_count(&mut self) -> usize {
        let Ok(key) = self.ensure_layout(self.cursor.chapter) else {
            return 1;
        };
        self.layouts
            .peek(&key)
            .map(|entry| entry.pagination.page_count())
            .unwrap_or(1)
    }

    fn turn_pages(&mut self, delta: i32) -> Result<()> {
        let page_count = self.page_count() as i64;
        let target = self.cursor.page as i64 + delta as i64;
        if target >= 0 && target < page_count {
            self.cursor.page = target as usize;
            self.spotlight_block = self.first_block_on_page();
            return Ok(());
        }
        if target >= page_count {
            // Forward past the end: first page of the next chapter.
            if self.cursor.chapter + 1 < self.chapter_count() {
                self.cursor.chapter += 1;
                self.cursor.page = 0;
                self.spotlight_block = self.first_block_on_page();
            }
            return Ok(());
        }
        // Backward past the start: last page of the previous chapter.
        if self.cursor.chapter > 0 {
            self.cursor.chapter -= 1;
            self.cursor.page = self.page_count().saturating_sub(1);
            self.spotlight_block = self.first_block_on_page();
        }
        Ok(())
    }

    fn replace_settings(&mut self, settings: TypeSettings) -> Result<()> {
        let anchor = self.position()?;
        let refresh_stack = settings.family != self.settings.family
            || settings.mono_family != self.settings.mono_family;
        self.settings = settings;
        if refresh_stack {
            self.stack = FontStack::resolve(
                &self.layouter.fonts,
                self.settings.family.as_deref(),
                self.settings.mono_family.as_deref(),
            );
        }
        self.go_to(anchor)
    }

    /// Move the cursor to a logical position, resolving it to a page.
    fn go_to(&mut self, position: Position) -> Result<()> {
        let chapter = position.chapter.min(self.chapter_count() - 1);
        self.cursor.chapter = chapter;
        let key = self.ensure_layout(chapter)?;
        let page = match self.layouts.peek(&key) {
            Some(entry) => {
                let block_index = entry.layout.position_of_block(position.block).unwrap_or(0);
                let line =
                    paginate::line_for_char(&entry.layout, block_index, position.char_offset);
                entry.pagination.page_of_line(block_index, line)
            }
            None => 0,
        };
        self.cursor.page = page;
        self.spotlight_block = self.first_block_on_page();
        Ok(())
    }

    /// Resolve a link target's fragment to a position.
    fn position_for_anchor(&mut self, chapter: usize, anchor: Option<&str>) -> Result<Position> {
        let Some(anchor) = anchor else {
            return Ok(Position::chapter_start(chapter));
        };
        let key = self.ensure_layout(chapter)?;
        let block = self
            .layouts
            .peek(&key)
            .and_then(|entry| {
                entry
                    .layout
                    .anchors
                    .get(anchor)
                    .and_then(|index| entry.layout.blocks.get(*index))
                    .map(|block| block.block)
            })
            .unwrap_or(0);
        Ok(Position {
            chapter,
            block,
            char_offset: 0,
        })
    }

    fn first_block_on_page(&mut self) -> usize {
        let Ok(key) = self.ensure_layout(self.cursor.chapter) else {
            return 0;
        };
        let page = self.cursor.page;
        self.layouts
            .peek(&key)
            .and_then(|entry| entry.pagination.pages.get(page))
            .and_then(|page| page.first_line())
            .map(|(block, _)| block)
            .unwrap_or(0)
    }

    fn move_spotlight(&mut self, delta: i32) -> Result<()> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let page_index = self.cursor.page;
        let blocks: Vec<usize> = self
            .layouts
            .peek(&key)
            .and_then(|entry| entry.pagination.pages.get(page_index))
            .map(|page| page.placements.iter().map(|p| p.block).collect())
            .unwrap_or_default();
        if blocks.is_empty() {
            return Ok(());
        }
        let current = blocks
            .iter()
            .position(|b| *b == self.spotlight_block)
            .unwrap_or(0) as i64;
        let next = current + delta as i64;
        if next < 0 {
            self.turn_pages(-1)?;
            self.spotlight_block = self.last_block_on_page();
            return Ok(());
        }
        if next as usize >= blocks.len() {
            self.turn_pages(1)?;
            self.spotlight_block = self.first_block_on_page();
            return Ok(());
        }
        self.spotlight_block = blocks[next as usize];
        Ok(())
    }

    fn last_block_on_page(&mut self) -> usize {
        let Ok(key) = self.ensure_layout(self.cursor.chapter) else {
            return 0;
        };
        let page = self.cursor.page;
        self.layouts
            .peek(&key)
            .and_then(|entry| entry.pagination.pages.get(page))
            .and_then(|page| page.placements.last())
            .map(|placement| placement.block)
            .unwrap_or(0)
    }

    /// Parse a chapter, or return the cached parse.
    fn chapter(&mut self, chapter: usize) -> Result<Arc<Chapter>> {
        if let Some(parsed) = self.chapters.get(&chapter) {
            return Ok(Arc::clone(parsed));
        }
        let xhtml = self.book.chapter_xhtml(chapter)?;
        let parsed = Arc::new(parse_chapter(&xhtml));
        if let Some(slot) = self.chapter_text.get_mut(chapter) {
            *slot = Some(Arc::from(parsed.text().as_str()));
        }
        self.chapters.put(chapter, Arc::clone(&parsed));
        Ok(parsed)
    }

    fn layout_key(&self, chapter: usize) -> LayoutKey {
        LayoutKey {
            chapter,
            width: self.viewport.0,
            height: self.viewport.1,
            narrowed: self.focus,
            type_key: self.settings.layout_key(),
        }
    }

    /// Geometry of the text column for the current viewport.
    fn geometry(&self) -> (f32, f32, f32, f32) {
        let (width, height) = (self.viewport.0 as f32, self.viewport.1 as f32);
        let column_width = self.settings.column_width(width, self.focus);
        let column_x = ((width - column_width) / 2.0).max(0.0);
        let content_y = if self.focus {
            self.settings.font_px * 1.4
        } else {
            self.settings.font_px * 1.0
        };
        let content_height = (height - content_y * 2.0).max(self.settings.font_px * 3.0);
        (column_width, column_x, content_y, content_height)
    }

    /// Lay out and paginate a chapter if that combination is not cached.
    fn ensure_layout(&mut self, chapter: usize) -> Result<LayoutKey> {
        let key = self.layout_key(chapter);
        if self.layouts.contains(&key) {
            return Ok(key);
        }
        let parsed = self.chapter(chapter)?;
        let (column_width, _, _, content_height) = self.geometry();

        let Self {
            book,
            layouter,
            images,
            layouts,
            settings,
            stack,
            ..
        } = self;

        let chapter_path = book.chapter_path(chapter).unwrap_or_default().to_string();
        let layout = layouter.layout_chapter(
            chapter,
            &parsed,
            settings,
            column_width,
            content_height,
            &stack.body,
            &stack.mono,
            |href| {
                let (raw, _) = href_path::split_fragment(href);
                let path = href_path::resolve(&chapter_path, &href_path::percent_decode(raw));
                if let Some(decoded) = images.get(&path) {
                    return Some(ImageInfo {
                        path,
                        width: decoded.width,
                        height: decoded.height,
                    });
                }
                let bytes = book.resource(&path).ok()?;
                let decoded = DecodedImage::decode(&bytes)?;
                let info = ImageInfo {
                    path: path.clone(),
                    width: decoded.width,
                    height: decoded.height,
                };
                if images.len() >= IMAGE_CACHE {
                    images.clear();
                }
                images.insert(path, decoded);
                Some(info)
            },
        );
        let pagination = paginate::paginate(&layout, content_height);
        layouts.put(key.clone(), LayoutEntry { layout, pagination });
        Ok(key)
    }

    /// Lay out the neighbouring chapters so crossing a boundary is instant.
    pub fn prefetch_neighbours(&mut self) {
        let chapter = self.cursor.chapter;
        if chapter + 1 < self.chapter_count() {
            let _ = self.ensure_layout(chapter + 1);
        }
        if chapter > 0 {
            let _ = self.ensure_layout(chapter - 1);
        }
        // Re-establish the current chapter as most recently used.
        let _ = self.ensure_layout(chapter);
    }

    /// Draw the current page.
    fn render(&mut self) -> Result<View> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let page_count = self.page_count();
        self.cursor.page = self.cursor.page.min(page_count.saturating_sub(1));
        let position = self.position()?;
        let progress = self.progress();
        let toc_index = self.active_toc();
        let chapter_label = self.current_label();
        let (column_width, column_x, content_y, _) = self.geometry();
        let frame = PageFrame {
            viewport_width: self.viewport.0,
            viewport_height: self.viewport.1,
            column_x,
            content_y,
            spotlight: (self.focus && self.spotlight).then_some(self.spotlight_block),
        };
        let page = self.cursor.page;
        let theme = self.theme;

        let Self {
            layouts,
            layouter,
            swash,
            images,
            ..
        } = self;
        let entry = layouts
            .get(&key)
            .ok_or_else(|| Error::Ebook("layout vanished from cache".into()))?;
        debug_assert!((entry.layout.column_width - column_width).abs() < 1.0);
        let rendered = render_page(
            &mut layouter.fonts,
            swash,
            &entry.layout,
            &entry.pagination,
            page,
            frame,
            &theme,
            images,
        );

        Ok(View {
            canvas: rendered.canvas,
            links: rendered.links,
            chapter: self.cursor.chapter,
            chapter_label,
            toc_index,
            page,
            page_count,
            position,
            progress,
            focus: self.focus,
            spotlight: self.spotlight,
            can_go_back: !self.history.is_empty(),
            font_px: self.settings.font_px,
        })
    }
}

/// A short excerpt around a match, with ellipses where it was cut.
fn snippet_around(text: &str, byte_index: usize, needle_len: usize) -> String {
    const CONTEXT: usize = 48;
    let start = text[..byte_index]
        .char_indices()
        .rev()
        .nth(CONTEXT)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let tail_start = (byte_index + needle_len).min(text.len());
    let end = text[tail_start..]
        .char_indices()
        .nth(CONTEXT)
        .map(|(i, _)| tail_start + i)
        .unwrap_or(text.len());
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.push_str(text[start..end].trim());
    if end < text.len() {
        snippet.push('…');
    }
    snippet
}
