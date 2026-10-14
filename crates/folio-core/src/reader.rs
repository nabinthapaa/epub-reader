//! The reading session: the single place that knows where the reader is.
//!
//! The UI sends [`Command`]s and gets a [`View`] back. Keeping the cursor,
//! caches and anchoring here means the whole reading model can be tested
//! headlessly, and the Slint layer stays a projection of this state.
//!
//! Reading is continuous inside a chapter: the cursor is a scroll offset in
//! chapter space, and the renderer paints a band taller than the viewport so
//! the UI can slide it without asking for new pixels. Crossing into another
//! chapter is the one discontinuity, and it is reported as a [`Transition`] so
//! the UI can animate it rather than snap.

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
    Canvas, ChapterLayout, DecodedImage, Flow, FontStack, Layouter, TypeSettings, flow,
    raster::{BandFrame, LinkRegion, render_band},
};
use crate::state::{Position, Settings};
use crate::theme::{Theme, ThemePreset};

/// How many laid-out chapters to keep (current plus one each side).
const LAYOUT_CACHE: usize = 3;
/// How many parsed chapters to keep.
const CHAPTER_CACHE: usize = 6;
/// Decoded images held for the open book before the map is recycled. Counted
/// in bytes: one 2000x2000 SVG decodes to 16MB, so a limit on the number of
/// entries bounds nothing that matters.
const IMAGE_CACHE_BYTES: usize = 96 * 1024 * 1024;
/// Band height as a multiple of the viewport.
const BAND_FACTOR: f32 = 2.2;
/// How far the view may be eased (rather than snapped) without running off the
/// painted band: half the pre-roll, with a little margin.
pub const EASE_LIMIT_FRACTION: f32 = (BAND_FACTOR - 1.0) / 2.0 * 0.85;
/// Ceiling on a band's pixels, so a 4K window does not paint (and ship to the
/// UI) tens of megabytes per repaint.
const BAND_BYTE_BUDGET: f32 = 32.0 * 1024.0 * 1024.0;
/// How close the viewport may come to a band edge before a new band is painted,
/// as a fraction of the viewport.
const BAND_MARGIN: f32 = 0.4;
/// Scroll step for the arrow keys, as a fraction of the viewport.
const LINE_STEP: f32 = 0.18;
/// Overlap kept when scrolling by a whole screen.
const PAGE_OVERLAP: f32 = 0.1;

/// What the UI can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Re-render where we are (after a theme change, for instance).
    Refresh,
    Resize {
        width: u32,
        height: u32,
    },
    /// Scroll by a number of pixels; positive is downwards. Scrolling past
    /// either end of a chapter crosses into the next or previous one.
    Scroll(f32),
    /// Scroll by whole screens, keeping a little overlap.
    ScrollScreens(f32),
    /// Scroll by one arrow-key step.
    ScrollSteps(f32),
    /// Jump to a fraction of the chapter, for dragging the scrollbar.
    ScrollTo(f32),
    /// Jump whole spine documents.
    Chapter(i32),
    /// Step to the next or previous table-of-contents entry.
    ///
    /// This is what the status bar's stepper sends, because the label beside it
    /// names a TOC entry. Most Gutenberg EPUBs put ten or more chapters in one
    /// spine document, so stepping documents there would skip most of the book
    /// the reader can see named in Contents.
    Section(i32),
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

/// Which way the reader crossed a chapter boundary, for the UI's animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Forward,
    Backward,
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
    /// Freshly painted band, or `None` when the previous band still covers the
    /// viewport and only the scroll offset moved.
    pub band: Option<Canvas>,
    /// Offset of the band's top in chapter space.
    pub band_top: f32,
    /// Where the viewport's top sits in chapter space.
    pub scroll: f32,
    pub chapter_height: f32,
    pub viewport_height: f32,
    /// Hit-testable links, in chapter space. Only sent with a new band.
    pub links: Option<Vec<LinkRegion>>,
    pub chapter: usize,
    pub chapter_label: String,
    /// Row of the table of contents the reader is inside, if any.
    pub toc_index: Option<usize>,
    pub position: Position,
    /// Fraction of the book read, 0.0..=1.0.
    pub progress: f32,
    /// Fraction of the current chapter above the viewport.
    pub chapter_progress: f32,
    /// Rough minutes of reading left in this section, when it is worth saying.
    pub minutes_left: Option<u32>,
    /// Set when this frame begins a new chapter.
    pub transition: Option<Transition>,
    pub focus: bool,
    pub spotlight: bool,
    pub can_go_back: bool,
    pub font_px: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cursor {
    chapter: usize,
    scroll: f32,
}

/// What a laid-out chapter actually depends on.
///
/// Keying on the raw window size meant every pixel of a resize drag produced a
/// fresh key — and so a full reshape of every block — even though the column is
/// pinned at the measure cap for any window wider than about 830px, and even
/// though the layouts were byte-identical.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LayoutKey {
    chapter: usize,
    /// Bits of the column width, which already folds in margins and focus mode.
    column_width: u32,
    /// Bits of the tallest block that can still fit a screen.
    max_block_height: u32,
    type_key: String,
}

struct LayoutEntry {
    layout: ChapterLayout,
    flow: Flow,
}

/// What the band currently on screen covers, so that plain scrolling inside it
/// costs nothing.
#[derive(Clone, Debug, PartialEq)]
struct Band {
    key: LayoutKey,
    top: f32,
    height: f32,
    theme: ThemePreset,
    spotlight: Option<usize>,
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
    band: Option<Band>,
    pending_transition: Option<Transition>,
    /// Characters per chapter, filled in as chapters are parsed. Used for the
    /// time-left estimate, which only ever concerns the chapter in hand.
    chapter_chars: Vec<Option<usize>>,
    /// Relative weight of every chapter, taken once at open from the size of
    /// each spine resource. Progress that counts spine entries is meaningless
    /// — a title page and a forty-page chapter weigh the same — but a weight
    /// that changes as chapters are parsed is worse: the same position would
    /// report different percentages from one frame to the next, and the number
    /// is persisted to the shelf. Markup overhead is roughly proportional to
    /// text, so byte sizes are a good enough and, crucially, stable ruler.
    chapter_weights: Vec<f32>,
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
                scroll: 0.0,
            },
            spotlight_block: 0,
            history: Vec::new(),
            chapters: LruCache::new(NonZeroUsize::new(CHAPTER_CACHE).unwrap()),
            layouts: LruCache::new(NonZeroUsize::new(LAYOUT_CACHE).unwrap()),
            images: HashMap::new(),
            band: None,
            pending_transition: None,
            chapter_chars: vec![None; chapter_count],
            chapter_weights: Vec::new(),
        };
        reader.chapter_weights = (0..chapter_count)
            .map(|chapter| reader.book.chapter_bytes(chapter).unwrap_or(0).max(1) as f32)
            .collect();
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

    /// The body family text is actually shaped with, after the preference chain
    /// has been resolved against what is installed.
    pub fn body_family(&self) -> String {
        self.stack.body.clone()
    }

    /// Available body font families, for the settings panel.
    pub fn font_families(&self) -> Vec<String> {
        crate::layout::installed_families(&self.layouter.fonts)
    }

    /// Apply a command and render the resulting frame.
    pub fn apply(&mut self, command: Command) -> Result<View> {
        self.mutate(command)?;
        self.render()
    }

    /// Apply a command without painting.
    ///
    /// The worker coalesces bursts of commands and paints only the last. A
    /// painted-but-discarded band would leave the reader believing the window
    /// shows pixels it never received, so intermediate commands skip the
    /// raster entirely — which also saves the work.
    pub fn apply_quietly(&mut self, command: Command) -> Result<()> {
        self.mutate(command)
    }

    fn mutate(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Refresh => {}
            Command::Resize { width, height } => {
                if self.viewport == (width, height) {
                    return Ok(());
                }
                let anchor = self.position()?;
                self.viewport = (width, height);
                self.band = None;
                self.go_to(anchor)?;
            }
            Command::Scroll(delta) => self.scroll_by(delta)?,
            Command::ScrollScreens(screens) => {
                let step = self.content_height() * (1.0 - PAGE_OVERLAP);
                self.scroll_by(screens * step)?;
            }
            Command::ScrollSteps(steps) => {
                let step = self.content_height() * LINE_STEP;
                self.scroll_by(steps * step)?;
            }
            Command::ScrollTo(fraction) if !fraction.is_finite() => {}
            Command::ScrollTo(fraction) => {
                let max = self.max_scroll()?;
                self.cursor.scroll = (fraction.clamp(0.0, 1.0) * max).clamp(0.0, max);
                self.spotlight_block = self.first_visible_block();
            }
            Command::Chapter(delta) => {
                let target = (self.cursor.chapter as i64 + delta as i64)
                    .clamp(0, self.chapter_count() as i64 - 1)
                    as usize;
                if target != self.cursor.chapter {
                    self.pending_transition = Some(if target > self.cursor.chapter {
                        Transition::Forward
                    } else {
                        Transition::Backward
                    });
                }
                self.go_to(Position::chapter_start(target))?;
            }
            Command::GoTo(position) => {
                let from = self.cursor.chapter;
                self.go_to(position)?;
                self.note_chapter_change(from);
            }
            Command::GoToToc(index) => self.go_to_toc(index, true)?,
            Command::Section(delta) => self.step_section(delta)?,
            Command::FollowLink(href) => {
                if let Some(Target { chapter, anchor }) =
                    self.book.resolve_link(self.cursor.chapter, &href)
                {
                    let from_position = self.position()?;
                    let from_chapter = self.cursor.chapter;
                    let target = self.position_for_anchor(chapter, anchor.as_deref())?;
                    self.history.push(from_position);
                    self.go_to(target)?;
                    self.note_chapter_change(from_chapter);
                }
            }
            Command::Back => {
                if let Some(previous) = self.history.pop() {
                    let from = self.cursor.chapter;
                    self.go_to(previous)?;
                    self.note_chapter_change(from);
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
            Command::SetTheme(theme) => {
                self.theme = theme;
                self.band = None;
            }
            Command::SetFocus(focus) => {
                if self.focus != focus {
                    let anchor = self.position()?;
                    self.focus = focus;
                    self.band = None;
                    self.go_to(anchor)?;
                }
            }
            Command::SetSpotlight(spotlight) => {
                self.spotlight = spotlight;
                self.band = None;
            }
            Command::MoveSpotlight(delta) => self.move_spotlight(delta)?,
        }
        Ok(())
    }

    /// Current position, derived from the first line in view.
    pub fn position(&mut self) -> Result<Position> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let scroll = self.cursor.scroll;
        let Some(entry) = self.layouts.peek(&key) else {
            return Ok(Position::chapter_start(self.cursor.chapter));
        };
        let (block_index, line) = entry.flow.first_line_at(&entry.layout, scroll);
        let block = entry
            .layout
            .blocks
            .get(block_index)
            .map(|b| b.block)
            .unwrap_or(0);
        let char_offset = char_for_line(&entry.layout, block_index, line);
        Ok(Position {
            chapter: self.cursor.chapter,
            block,
            char_offset,
        })
    }

    /// Fraction of the book behind the current viewport, weighted by how long
    /// each chapter is rather than by how many there are.
    pub fn progress(&mut self) -> f32 {
        let within = self.chapter_fraction();
        let current = self.cursor.chapter;

        let total: f32 = self.chapter_weights.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            let chapters = self.chapter_count().max(1) as f32;
            return ((current as f32 + within) / chapters).clamp(0.0, 1.0);
        }
        let behind: f32 = self.chapter_weights.iter().take(current).sum::<f32>()
            + within * self.chapter_weights.get(current).copied().unwrap_or(0.0);
        (behind / total).clamp(0.0, 1.0)
    }

    /// Minutes of reading left in the current chapter at ~250 words a minute,
    /// or `None` when it is less than a minute and not worth saying.
    pub fn minutes_left(&mut self) -> Option<u32> {
        let chars = self.section_chars_left()?;
        let minutes = (chars as f32 / 5.0 / 250.0).round() as u32;
        (minutes >= 1).then_some(minutes)
    }

    /// Characters between the top of the viewport and the end of the section
    /// the status bar is naming.
    ///
    /// Scoped to the table-of-contents entry rather than the spine document,
    /// because that is what the label beside it says. Gutenberg's Moby Dick is
    /// twelve documents of about ninety reading-minutes each, so a
    /// document-scoped estimate sits beside "CHAPTER 9. The Sermon." claiming
    /// an hour and a half for a ten-minute chapter.
    fn section_chars_left(&mut self) -> Option<usize> {
        let chapter = self.cursor.chapter;
        // Measured from the bottom of the viewport: what is already on screen
        // is not time left, and counting it would never let the estimate reach
        // zero at the end of a section.
        let from = self.last_visible_block();
        let key = self.ensure_layout(chapter).ok()?;
        let entry = self.layouts.peek(&key)?;

        // The next entry that starts below us ends the section; without one the
        // section runs to the end of the document.
        let mut end = usize::MAX;
        for item in self.book.toc() {
            let Some(target) = &item.target else { continue };
            if target.chapter != chapter || item.label.trim().is_empty() {
                continue;
            }
            let Some(anchor) = target.anchor.as_deref() else {
                continue;
            };
            if let Some(&block) = entry.layout.anchors.get(anchor)
                && block > from
                && block < end
            {
                end = block;
            }
        }

        let parsed = self.chapter(chapter).ok()?;
        let end = end.min(parsed.blocks.len());
        Some(
            parsed
                .blocks
                .get(from..end)?
                .iter()
                .map(|block| block.text().chars().count())
                .sum(),
        )
    }

    /// How far through the current chapter the viewport is.
    pub fn chapter_fraction(&mut self) -> f32 {
        let viewport = self.content_height();
        let Ok(key) = self.ensure_layout(self.cursor.chapter) else {
            return 0.0;
        };
        let scroll = self.cursor.scroll;
        self.layouts
            .peek(&key)
            .map(|entry| entry.flow.fraction(scroll, viewport))
            .unwrap_or(0.0)
    }

    /// Search the whole book, parsing chapters on demand.
    pub fn search(&mut self, query: &str, limit: usize) -> Vec<SearchHit> {
        let needle: Vec<char> = query.trim().to_lowercase().chars().collect();
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
                if let Some((start, end)) = find_case_insensitive(&text, &needle) {
                    hits.push(SearchHit {
                        position: Position {
                            chapter,
                            block: block_index,
                            char_offset: start,
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
                        snippet: snippet_around(&text, start, end),
                    });
                }
            }
        }
        hits
    }

    /// Jump to a table-of-contents entry, honoring its fragment.
    ///
    /// `push_history` is false for the status bar's stepper: the back stack
    /// exists so a followed footnote can be undone, and filling it with every
    /// chapter step would make `Backspace` mean two different things.
    fn go_to_toc(&mut self, index: usize, push_history: bool) -> Result<()> {
        let Some(target) = self
            .book
            .toc()
            .get(index)
            .and_then(|item| item.target.clone())
        else {
            return Ok(());
        };
        let from_position = self.position()?;
        let from_chapter = self.cursor.chapter;
        let position = self.position_for_anchor(target.chapter, target.anchor.as_deref())?;
        if push_history {
            self.history.push(from_position);
        }
        self.go_to(position)?;
        self.note_chapter_change(from_chapter);
        Ok(())
    }

    /// Step the table of contents by `delta` entries from the one we are in.
    ///
    /// Books with no usable table of contents fall back to stepping spine
    /// documents, which is the best "next chapter" such a book can offer.
    fn step_section(&mut self, delta: i32) -> Result<()> {
        let usable = self.usable_toc();
        if usable.len() < 2 {
            return self.mutate(Command::Chapter(delta));
        }
        let here = self
            .active_toc()
            .and_then(|current| usable.iter().position(|&index| index == current))
            .unwrap_or(0);
        let target = (here as i64 + delta as i64).clamp(0, usable.len() as i64 - 1) as usize;
        if target == here {
            return Ok(());
        }
        self.go_to_toc(usable[target], false)
    }

    /// Table-of-contents entries that can actually be navigated to.
    fn usable_toc(&self) -> Vec<usize> {
        self.book
            .toc()
            .iter()
            .enumerate()
            .filter(|(_, item)| item.target.is_some() && !item.label.trim().is_empty())
            .map(|(index, _)| index)
            .collect()
    }

    /// The table-of-contents row the viewport sits inside.
    ///
    /// Books that put many chapters in one spine file (most Gutenberg EPUBs do)
    /// need the entry's fragment resolved against the laid-out blocks, not just
    /// its spine index, or every page claims to be the file's last chapter.
    pub fn active_toc(&mut self) -> Option<usize> {
        let chapter = self.cursor.chapter;
        let current_block = self.first_visible_block();
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
        // Front matter matches no entry at all; pointing at the first one is
        // better than leaving the panel with no "you are here" whatsoever.
        best.map(|(index, _)| index).or(fallback).or_else(|| {
            self.book
                .toc()
                .iter()
                .position(|item| !item.label.trim().is_empty())
        })
    }

    /// Human-readable label for what is on screen.
    pub fn current_label(&mut self) -> String {
        let chapter = self.cursor.chapter;
        self.active_toc()
            .and_then(|index| self.book.toc().get(index))
            .map(|item| item.label.clone())
            .unwrap_or_else(|| {
                if chapter == 0 {
                    self.book.meta().title.clone()
                } else {
                    format!("Chapter {}", chapter + 1)
                }
            })
    }

    // --- navigation --------------------------------------------------------

    /// Scroll within the chapter, crossing into the next or previous one when
    /// the reader is already at an edge and keeps going.
    fn scroll_by(&mut self, delta: f32) -> Result<()> {
        // A non-finite delta would be absorbed by the clamp below and then
        // poison every later clamp, wedging the reader for the session.
        if !delta.is_finite() || delta == 0.0 {
            return Ok(());
        }
        let max = self.max_scroll()?;
        let at_end = self.cursor.scroll >= max - 0.5;
        let at_start = self.cursor.scroll <= 0.5;

        if delta > 0.0 && at_end {
            if self.cursor.chapter + 1 < self.chapter_count() {
                self.pending_transition = Some(Transition::Forward);
                self.cursor.chapter += 1;
                self.cursor.scroll = 0.0;
                self.band = None;
                self.spotlight_block = self.first_visible_block();
            }
            return Ok(());
        }
        if delta < 0.0 && at_start {
            if self.cursor.chapter > 0 {
                self.pending_transition = Some(Transition::Backward);
                self.cursor.chapter -= 1;
                self.band = None;
                let max = self.max_scroll()?;
                self.cursor.scroll = max;
                self.spotlight_block = self.first_visible_block();
            }
            return Ok(());
        }

        self.cursor.scroll = (self.cursor.scroll + delta).clamp(0.0, max);
        self.spotlight_block = self.first_visible_block();
        Ok(())
    }

    /// Record a transition when a jump landed in a different chapter.
    fn note_chapter_change(&mut self, from: usize) {
        if self.cursor.chapter == from {
            return;
        }
        self.pending_transition = Some(if self.cursor.chapter > from {
            Transition::Forward
        } else {
            Transition::Backward
        });
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
        self.band = None;
        self.go_to(anchor)
    }

    /// Move the cursor to a logical position, resolving it to a scroll offset.
    fn go_to(&mut self, position: Position) -> Result<()> {
        let chapter = position.chapter.min(self.chapter_count() - 1);
        if chapter != self.cursor.chapter {
            self.band = None;
        }
        self.cursor.chapter = chapter;
        let key = self.ensure_layout(chapter)?;
        let viewport = self.content_height();
        let scroll = match self.layouts.peek(&key) {
            Some(entry) => {
                let block_index = entry.layout.position_of_block(position.block).unwrap_or(0);
                let line = line_for_char(&entry.layout, block_index, position.char_offset);
                let y = entry.flow.line_y(&entry.layout, block_index, line);
                // Show a little space above the target, but never so much that
                // the previous block creeps back into view: the line we aimed
                // at must still be the first one visible, or the position would
                // not round-trip and the contents panel would lag a row behind.
                let target = if block_index == 0 && line == 0 {
                    0.0
                } else if line == 0 {
                    let previous_bottom = entry
                        .flow
                        .placements
                        .iter()
                        .rfind(|p| p.block < block_index)
                        .map(|p| p.bottom())
                        .unwrap_or(0.0);
                    (y - entry.flow.pad).clamp(previous_bottom, y)
                } else {
                    y
                };
                target.clamp(0.0, entry.flow.max_scroll(viewport))
            }
            None => 0.0,
        };
        self.cursor.scroll = scroll;
        self.spotlight_block = self.first_visible_block();
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

    /// Index into the layout's blocks of the first block in view.
    fn first_visible_block(&mut self) -> usize {
        let Ok(key) = self.ensure_layout(self.cursor.chapter) else {
            return 0;
        };
        let scroll = self.cursor.scroll;
        self.layouts
            .peek(&key)
            .map(|entry| entry.flow.first_line_at(&entry.layout, scroll).0)
            .unwrap_or(0)
    }

    /// The block under the bottom edge of the viewport.
    fn last_visible_block(&mut self) -> usize {
        let bottom = self.cursor.scroll + self.content_height();
        let Ok(key) = self.ensure_layout(self.cursor.chapter) else {
            return 0;
        };
        self.layouts
            .peek(&key)
            .map(|entry| entry.flow.first_line_at(&entry.layout, bottom).0)
            .unwrap_or(0)
    }

    /// Walk the spotlight by whole paragraphs, scrolling to keep it in view.
    fn move_spotlight(&mut self, delta: i32) -> Result<()> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let viewport = self.content_height();
        let current = self.spotlight_block;

        let Some(entry) = self.layouts.peek(&key) else {
            return Ok(());
        };
        let order: Vec<usize> = entry.flow.placements.iter().map(|p| p.block).collect();
        let index = order.iter().position(|b| *b == current).unwrap_or(0) as i64;
        let next = index + delta as i64;

        if next < 0 || next as usize >= order.len() {
            // Past either end of the chapter: let the scroll logic cross over.
            return self.scroll_by(if delta > 0 { viewport } else { -viewport });
        }

        let block = order[next as usize];
        let (y, height) = entry
            .flow
            .placements
            .iter()
            .find(|p| p.block == block)
            .map(|p| (p.y, p.height))
            .unwrap_or((0.0, 0.0));
        let max = entry.flow.max_scroll(viewport);
        self.spotlight_block = block;

        // Keep the active paragraph comfortably inside the viewport.
        let top_margin = viewport * 0.25;
        if y < self.cursor.scroll + top_margin || y + height > self.cursor.scroll + viewport {
            self.cursor.scroll = (y - top_margin).clamp(0.0, max);
        }
        Ok(())
    }

    // --- layout and painting ----------------------------------------------

    /// Parse a chapter, or return the cached parse.
    fn chapter(&mut self, chapter: usize) -> Result<Arc<Chapter>> {
        if let Some(parsed) = self.chapters.get(&chapter) {
            return Ok(Arc::clone(parsed));
        }
        let xhtml = self.book.chapter_xhtml(chapter)?;
        let parsed = Arc::new(parse_chapter(&xhtml));
        if let Some(slot) = self.chapter_chars.get_mut(chapter) {
            // Characters, not bytes: `minutes_left` divides by words, and a
            // Cyrillic or Greek book would otherwise weigh twice its length.
            *slot = Some(
                parsed
                    .blocks
                    .iter()
                    .map(|block| block.text().chars().count())
                    .sum(),
            );
        }
        self.chapters.put(chapter, Arc::clone(&parsed));
        Ok(parsed)
    }

    fn layout_key(&self, chapter: usize) -> LayoutKey {
        let (column_width, _, pad) = self.geometry();
        LayoutKey {
            chapter,
            column_width: column_width.to_bits(),
            max_block_height: (self.content_height() - 2.0 * pad).to_bits(),
            type_key: self.settings.layout_key(),
        }
    }

    /// Geometry of the text column: (column width, left edge, vertical pad).
    fn geometry(&self) -> (f32, f32, f32) {
        let width = self.viewport.0 as f32;
        let column_width = self.settings.column_width(width, self.focus);
        let column_x = ((width - column_width) / 2.0).max(0.0);
        let pad = if self.focus {
            self.settings.font_px * 2.2
        } else {
            self.settings.font_px * 1.6
        };
        (column_width, column_x, pad)
    }

    /// Height of the viewport, which is the whole window: the chrome is drawn
    /// by Slint around it.
    fn content_height(&self) -> f32 {
        self.viewport.1 as f32
    }

    fn max_scroll(&mut self) -> Result<f32> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let viewport = self.content_height();
        Ok(self
            .layouts
            .peek(&key)
            .map(|entry| entry.flow.max_scroll(viewport))
            .unwrap_or(0.0))
    }

    /// Lay out and flow a chapter if that combination is not cached.
    fn ensure_layout(&mut self, chapter: usize) -> Result<LayoutKey> {
        let key = self.layout_key(chapter);
        if self.layouts.contains(&key) {
            return Ok(key);
        }
        let parsed = self.chapter(chapter)?;
        let (column_width, _, pad) = self.geometry();
        let viewport_height = self.content_height();

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
            // An image taller than the window is scaled to fit it, so a figure
            // never needs two screens of scrolling to see at once.
            viewport_height - 2.0 * pad,
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
                let held: usize = images.values().map(|image| image.rgba.len()).sum();
                if held + decoded.rgba.len() > IMAGE_CACHE_BYTES {
                    images.clear();
                }
                images.insert(path, decoded);
                Some(info)
            },
        );
        let flowed = flow(&layout, pad);
        layouts.put(
            key.clone(),
            LayoutEntry {
                layout,
                flow: flowed,
            },
        );
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

    /// Decide whether the painted band still covers the viewport.
    fn band_is_usable(&self, key: &LayoutKey, chapter_height: f32) -> bool {
        let Some(band) = &self.band else { return false };
        if band.key != *key
            || band.theme != self.theme.preset
            || band.spotlight != self.spotlight_target()
        {
            return false;
        }
        let viewport = self.content_height();
        let margin = viewport * BAND_MARGIN;
        let top_ok = band.top <= 0.0 || self.cursor.scroll >= band.top + margin;
        let band_bottom = band.top + band.height;
        let bottom_ok =
            band_bottom >= chapter_height || self.cursor.scroll + viewport <= band_bottom - margin;
        top_ok && bottom_ok
    }

    fn spotlight_target(&self) -> Option<usize> {
        (self.focus && self.spotlight).then_some(self.spotlight_block)
    }

    /// Paint the band around the viewport, or reuse the one already on screen.
    fn render(&mut self) -> Result<View> {
        let key = self.ensure_layout(self.cursor.chapter)?;
        let viewport = self.content_height();
        let chapter_height = self
            .layouts
            .peek(&key)
            .map(|entry| entry.flow.height)
            .unwrap_or(viewport);
        let max_scroll = (chapter_height - viewport).max(0.0);
        self.cursor.scroll = self.cursor.scroll.clamp(0.0, max_scroll);

        let position = self.position()?;
        let progress = self.progress();
        let chapter_progress = self.chapter_fraction();
        let minutes_left = self.minutes_left();
        let toc_index = self.active_toc();
        let chapter_label = self.current_label();
        let transition = self.pending_transition.take();
        let (_, column_x, _) = self.geometry();

        if self.band_is_usable(&key, chapter_height) {
            let band = self.band.clone().expect("checked above");
            return Ok(self.view(
                None,
                band.top,
                chapter_height,
                None,
                position,
                progress,
                chapter_progress,
                minutes_left,
                toc_index,
                chapter_label,
                transition,
            ));
        }

        // Paint a band taller than the viewport, centred on it, so scrolling
        // can slide pixels rather than ask for new ones.
        // The band is useless unless it covers the viewport plus a margin at
        // each end — below that, every scroll repaints, which is the one thing
        // the band exists to avoid. So the byte budget caps the band but can
        // never shrink it past that floor: on a 5K display a bigger band is
        // cheaper than sixty repaints a second.
        let usable = chapter_height.max(viewport);
        let floor = (viewport * (1.0 + 2.0 * BAND_MARGIN)).min(usable);
        let budget_height = (BAND_BYTE_BUDGET / (self.viewport.0.max(1) as f32 * 4.0)).max(floor);
        let band_height = (viewport * BAND_FACTOR).min(usable).min(budget_height);
        let band_top = (self.cursor.scroll - (band_height - viewport) / 2.0)
            .clamp(0.0, (chapter_height - band_height).max(0.0));
        let frame = BandFrame {
            width: self.viewport.0,
            top: band_top,
            height: band_height.ceil().max(1.0) as u32,
            column_x,
            spotlight: self.spotlight_target(),
        };
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
        let rendered = render_band(
            &mut layouter.fonts,
            swash,
            &entry.layout,
            &entry.flow,
            frame,
            &theme,
            images,
        );

        self.band = Some(Band {
            key,
            top: band_top,
            height: frame.height as f32,
            theme: theme.preset,
            spotlight: frame.spotlight,
        });

        Ok(self.view(
            Some(rendered.canvas),
            band_top,
            chapter_height,
            Some(rendered.links),
            position,
            progress,
            chapter_progress,
            minutes_left,
            toc_index,
            chapter_label,
            transition,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn view(
        &self,
        band: Option<Canvas>,
        band_top: f32,
        chapter_height: f32,
        links: Option<Vec<LinkRegion>>,
        position: Position,
        progress: f32,
        chapter_progress: f32,
        minutes_left: Option<u32>,
        toc_index: Option<usize>,
        chapter_label: String,
        transition: Option<Transition>,
    ) -> View {
        View {
            band,
            band_top,
            scroll: self.cursor.scroll,
            chapter_height,
            viewport_height: self.content_height(),
            links,
            chapter: self.cursor.chapter,
            chapter_label,
            toc_index,
            position,
            progress,
            chapter_progress,
            minutes_left,
            transition,
            focus: self.focus,
            spotlight: self.spotlight,
            can_go_back: !self.history.is_empty(),
            font_px: self.settings.font_px,
        }
    }
}

/// Which visual line of a block contains `char_offset` of its text.
fn line_for_char(layout: &ChapterLayout, block: usize, char_offset: usize) -> usize {
    let Some(block_layout) = layout.blocks.get(block) else {
        return 0;
    };
    let Some(buffer) = block_layout.buffer.as_ref() else {
        return 0;
    };
    let mut last = 0;
    for (run_index, run) in buffer.layout_runs().enumerate() {
        last = run_index;
        if let (Some(first), Some(end)) = (run.glyphs.first(), run.glyphs.last())
            && char_offset >= first.start
            && char_offset < end.end
        {
            return run_index;
        }
    }
    last
}

/// Character offset of the first glyph on a line, for recording a position.
fn char_for_line(layout: &ChapterLayout, block: usize, line: usize) -> usize {
    layout
        .blocks
        .get(block)
        .and_then(|b| b.buffer.as_ref())
        .and_then(|buffer| {
            buffer
                .layout_runs()
                .nth(line)
                .and_then(|run| run.glyphs.first().map(|g| g.start))
        })
        .unwrap_or(0)
}

/// Find `needle` (already lowercased, as chars) in `text`, ignoring case, and
/// return the match as a range of **character** indices.
///
/// Case folding is not length-preserving — `'\u{212B}'` (3 bytes) lowercases to
/// `'å'` (2), `'\u{130}'` (2) to a 3-byte sequence — so searching a lowercased
/// copy and slicing the original with the offsets it returns lands inside a
/// character and panics. Walking the original text sidesteps that entirely.
fn find_case_insensitive(text: &str, needle: &[char]) -> Option<(usize, usize)> {
    let haystack: Vec<char> = text.chars().collect();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    for start in 0..=haystack.len() - needle.len() {
        let matched = haystack[start..start + needle.len()]
            .iter()
            .zip(needle)
            .all(|(found, wanted)| {
                found == wanted || found.to_lowercase().eq(std::iter::once(*wanted))
            });
        if matched {
            return Some((start, start + needle.len()));
        }
    }
    None
}

/// A short excerpt around a match, with ellipses where it was cut. `start` and
/// `end` are character indices.
fn snippet_around(text: &str, start: usize, end: usize) -> String {
    const CONTEXT: usize = 48;
    let from = start.saturating_sub(CONTEXT);
    let to = (end + CONTEXT).min(text.chars().count());
    let excerpt: String = text.chars().skip(from).take(to - from).collect();

    let mut snippet = String::new();
    if from > 0 {
        snippet.push('…');
    }
    snippet.push_str(excerpt.trim());
    if to < text.chars().count() {
        snippet.push('…');
    }
    snippet
}
