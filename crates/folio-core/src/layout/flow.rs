//! Laying a chapter out as one continuous column.
//!
//! Reading is vertical and unbroken inside a chapter, so blocks get absolute
//! offsets in chapter space and the renderer paints whichever band of that
//! column is near the viewport. Nothing here re-shapes text: it is arithmetic
//! over the line boxes layout already produced.

use super::engine::ChapterLayout;

/// Where one block sits in the chapter's column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockPlacement {
    /// Index into [`ChapterLayout::blocks`].
    pub block: usize,
    /// Offset of the block's first line from the top of the chapter.
    pub y: f32,
    pub height: f32,
}

impl BlockPlacement {
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

/// A chapter as a single scrollable column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub placements: Vec<BlockPlacement>,
    /// Total height including the padding at both ends.
    pub height: f32,
    /// Padding above the first block and below the last.
    pub pad: f32,
}

impl Flow {
    /// Offset of a block, by its index into the layout.
    pub fn block_y(&self, block: usize) -> Option<f32> {
        self.placements
            .iter()
            .find(|p| p.block == block)
            .map(|p| p.y)
    }

    /// Offset of one visual line inside the chapter.
    pub fn line_y(&self, layout: &ChapterLayout, block: usize, line: usize) -> f32 {
        let base = self.block_y(block).unwrap_or(0.0);
        let offset = layout
            .blocks
            .get(block)
            .and_then(|b| b.lines.get(line))
            .map(|l| l.top)
            .unwrap_or(0.0);
        base + offset
    }

    /// Blocks intersecting the band `[top, bottom)`.
    pub fn visible(&self, top: f32, bottom: f32) -> impl Iterator<Item = &BlockPlacement> {
        self.placements
            .iter()
            .filter(move |p| p.bottom() > top && p.y < bottom)
    }

    /// The first line at or below `scroll`, which is what a reading position
    /// records so it can survive a change of font size or window width.
    pub fn first_line_at(&self, layout: &ChapterLayout, scroll: f32) -> (usize, usize) {
        let mut fallback = (0usize, 0usize);
        for placement in &self.placements {
            let Some(block) = layout.blocks.get(placement.block) else {
                continue;
            };
            if placement.bottom() <= scroll {
                fallback = (placement.block, block.lines.len().saturating_sub(1));
                continue;
            }
            for (index, line) in block.lines.iter().enumerate() {
                if placement.y + line.top + line.height > scroll {
                    return (placement.block, index);
                }
            }
            return (placement.block, block.lines.len().saturating_sub(1));
        }
        fallback
    }

    /// How far down the chapter `scroll` is, as a fraction.
    pub fn fraction(&self, scroll: f32, viewport: f32) -> f32 {
        let scrollable = (self.height - viewport).max(1.0);
        (scroll / scrollable).clamp(0.0, 1.0)
    }

    /// Largest scroll offset that still shows content.
    pub fn max_scroll(&self, viewport: f32) -> f32 {
        (self.height - viewport).max(0.0)
    }
}

/// Lay the chapter's blocks into one column, `pad` px of breathing room at each
/// end. Space between blocks collapses the way it does in print: the larger of
/// the previous block's space-after and the next one's space-before.
pub fn flow(layout: &ChapterLayout, pad: f32) -> Flow {
    let mut placements = Vec::with_capacity(layout.blocks.len());
    let mut y = pad;
    let mut previous_space_after = 0.0_f32;

    for (index, block) in layout.blocks.iter().enumerate() {
        if block.lines.is_empty() {
            continue;
        }
        if !placements.is_empty() {
            y += previous_space_after.max(block.space_before);
        }
        placements.push(BlockPlacement {
            block: index,
            y,
            height: block.height,
        });
        y += block.height;
        previous_space_after = block.space_after;
    }

    Flow {
        height: y + pad,
        placements,
        pad,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_chapter_still_has_its_padding() {
        let layout = ChapterLayout {
            chapter: 0,
            column_width: 100.0,
            blocks: Vec::new(),
            anchors: Default::default(),
            block_text: Vec::new(),
        };
        let flow = flow(&layout, 24.0);
        assert_eq!(flow.height, 48.0);
        assert_eq!(flow.max_scroll(600.0), 0.0);
    }
}
