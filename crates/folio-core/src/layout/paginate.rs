//! Slicing a laid-out chapter into pages.
//!
//! Pages are runs of lines, never re-shaped text: pagination is pure
//! arithmetic over the line boxes the layout already produced.

use super::engine::ChapterLayout;

/// A contiguous run of one block's lines placed on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Index into [`ChapterLayout::blocks`].
    pub block: usize,
    pub line_from: usize,
    pub line_to: usize,
    /// Offset of the first line's top from the top of the page content area.
    pub y: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub placements: Vec<Placement>,
}

impl Page {
    /// First (block, line) on the page, used to anchor zoom changes.
    pub fn first_line(&self) -> Option<(usize, usize)> {
        self.placements.first().map(|p| (p.block, p.line_from))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pagination {
    pub pages: Vec<Page>,
    pub content_height: f32,
}

impl Pagination {
    pub fn page_count(&self) -> usize {
        self.pages.len().max(1)
    }

    /// Page showing a given line, or the last page if it is out of range.
    pub fn page_of_line(&self, block: usize, line: usize) -> usize {
        for (index, page) in self.pages.iter().enumerate() {
            for placement in &page.placements {
                if placement.block == block
                    && line >= placement.line_from
                    && line < placement.line_to
                {
                    return index;
                }
            }
        }
        // Blocks are in order, so fall back to the first page that starts at or
        // after the requested block.
        self.pages
            .iter()
            .position(|page| page.placements.first().is_some_and(|p| p.block >= block))
            .unwrap_or(self.pages.len().saturating_sub(1))
    }

    /// Page showing a block (its first visible line).
    pub fn page_of_block(&self, block: usize) -> usize {
        self.page_of_line(block, 0)
    }
}

/// Lay the chapter's lines into pages of `content_height` px.
pub fn paginate(layout: &ChapterLayout, content_height: f32) -> Pagination {
    let content_height = content_height.max(1.0);
    let mut pages: Vec<Page> = Vec::new();
    let mut page = Page::default();
    let mut y = 0.0_f32;

    for (block_index, block) in layout.blocks.iter().enumerate() {
        if block.lines.is_empty() {
            continue;
        }
        // Space above a block collapses at the top of a page.
        if !page.placements.is_empty() {
            y += block.space_before;
        }

        let mut pending: Option<(usize, f32)> = None;
        for (line_index, line) in block.lines.iter().enumerate() {
            let page_is_empty = page.placements.is_empty() && pending.is_none();
            if y + line.height > content_height && !page_is_empty {
                if let Some((start, start_y)) = pending.take() {
                    page.placements.push(Placement {
                        block: block_index,
                        line_from: start,
                        line_to: line_index,
                        y: start_y,
                    });
                }
                pages.push(std::mem::take(&mut page));
                y = 0.0;
            }
            if pending.is_none() {
                pending = Some((line_index, y));
            }
            y += line.height;
        }

        if let Some((start, start_y)) = pending.take() {
            page.placements.push(Placement {
                block: block_index,
                line_from: start,
                line_to: block.lines.len(),
                y: start_y,
            });
        }
        y += block.space_after;
    }

    if !page.placements.is_empty() {
        pages.push(page);
    }
    if pages.is_empty() {
        pages.push(Page::default());
    }

    Pagination {
        pages,
        content_height,
    }
}

/// Which visual line of a block contains `char_offset` of its text.
///
/// Used to turn a stored [`crate::state::Position`] into a page, and back.
pub fn line_for_char(layout: &ChapterLayout, block: usize, char_offset: usize) -> usize {
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
pub fn char_for_line(layout: &ChapterLayout, block: usize, line: usize) -> usize {
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
