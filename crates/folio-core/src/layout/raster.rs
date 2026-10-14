//! Drawing a band of the chapter's column into a [`Canvas`].
//!
//! Scrolling does not repaint: the renderer paints a band taller than the
//! viewport and the UI slides it, asking for a new band only when an edge comes
//! near. Nothing here re-shapes text, which is why a theme switch or a
//! spotlight move costs a raster and not a layout.

use std::collections::HashMap;

use cosmic_text::{FontSystem, SwashCache};

use super::canvas::Canvas;
use super::engine::{BlockLayout, ChapterLayout};
use super::flow::Flow;
use super::image::DecodedImage;
use crate::content::BlockKind;
use crate::theme::{Color, Theme};

/// Which slice of the chapter to paint, and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandFrame {
    pub width: u32,
    /// Offset of the band's top in chapter space.
    pub top: f32,
    pub height: u32,
    /// Left edge of the text column.
    pub column_x: f32,
    /// When set, every block but this one is dimmed (focus-mode spotlight).
    pub spotlight: Option<usize>,
}

/// A hit-testable region, in viewport x and chapter-space y, so that it stays
/// valid however far the reader has scrolled.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkRegion {
    pub href: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LinkRegion {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// A painted band plus what can be clicked inside it.
pub struct RenderedBand {
    pub canvas: Canvas,
    pub links: Vec<LinkRegion>,
}

/// Paint the band described by `frame`.
pub fn render_band(
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    layout: &ChapterLayout,
    flow: &Flow,
    frame: BandFrame,
    theme: &Theme,
    images: &HashMap<String, DecodedImage>,
) -> RenderedBand {
    let mut canvas = Canvas::new(frame.width, frame.height, theme.page);
    let mut links = Vec::new();
    let band_bottom = frame.top + frame.height as f32;

    for placement in flow.visible(frame.top, band_bottom) {
        let Some(block) = layout.blocks.get(placement.block) else {
            continue;
        };
        let dim = match frame.spotlight {
            Some(active) if active != placement.block => theme.dim_factor,
            _ => 1.0,
        };
        let x = frame.column_x + block.indent;
        // Chapter space -> band space.
        let y = placement.y - frame.top;

        match &block.kind {
            BlockKind::Rule => {
                let thickness = (block.height * 0.25).max(1.0);
                canvas.fill_rect(
                    x,
                    y + block.height / 2.0,
                    layout.column_width - block.indent,
                    thickness,
                    theme.border.fade(dim),
                );
                continue;
            }
            BlockKind::Quote { .. } => {
                let bar_width = (block.indent * 0.12).max(2.0);
                canvas.fill_rect(
                    x - block.indent * 0.45,
                    y,
                    bar_width,
                    block.height,
                    theme.accent.fade(dim * 0.8),
                );
            }
            _ => {}
        }

        if let Some(image) = &block.image {
            // The pixels can be evicted after the layout was built, so
            // "available" is a claim about layout time, not about now.
            let painted = image.available
                && draw_image(&mut canvas, image, x, y, layout.column_width, images, dim);
            if painted {
                continue;
            }
            // Unavailable: frame the space, then fall through to the alt text.
            canvas.stroke_dashed_rect(
                x,
                y,
                layout.column_width - block.indent,
                block.height,
                theme.border.fade(dim),
                6,
            );
        }

        draw_block_text(
            &mut canvas,
            fonts,
            swash,
            block,
            x,
            y,
            placement.y,
            theme,
            dim,
            &mut links,
        );
    }

    RenderedBand { canvas, links }
}

#[allow(clippy::too_many_arguments)]
fn draw_block_text(
    canvas: &mut Canvas,
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    block: &BlockLayout,
    x: f32,
    band_y: f32,
    chapter_y: f32,
    theme: &Theme,
    dim: f32,
    links: &mut Vec<LinkRegion>,
) {
    let Some(buffer) = block.buffer.as_ref() else {
        return;
    };
    for run in buffer.layout_runs() {
        let baseline = band_y + run.line_y;
        for glyph in run.glyphs {
            let role = block.roles.get(glyph.metadata);
            let base = match role {
                Some(role) if role.link.is_some() => theme.link,
                Some(role) if role.muted => theme.muted,
                _ => theme.text,
            };
            if let Some(href) = role.and_then(|r| r.link.as_ref()) {
                push_link_region(
                    links,
                    href,
                    x + glyph.x,
                    chapter_y + run.line_top,
                    glyph.w,
                    run.line_height,
                );
            }
            let physical = glyph.physical((x, baseline), 1.0);
            let color = cosmic_text::Color::rgba(base.r, base.g, base.b, base.a);
            swash.with_pixels(fonts, physical.cache_key, color, |px, py, pixel| {
                let (r, g, b, a) = pixel.as_rgba_tuple();
                canvas.blend(
                    physical.x + px,
                    physical.y + py,
                    Color::rgba(r, g, b, a).fade(dim),
                );
            });
        }
    }
}

/// Merge adjacent glyph boxes of the same link into one region.
fn push_link_region(
    links: &mut Vec<LinkRegion>,
    href: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    if let Some(last) = links.last_mut()
        && last.href == href
        && (last.y - y).abs() < 0.5
        && (last.x + last.width - x).abs() < 2.0
    {
        last.width += width;
        return;
    }
    links.push(LinkRegion {
        href: href.to_string(),
        x,
        y,
        width,
        height,
    });
}

/// Returns false when the pixels are no longer in the cache, so the caller can
/// fall back to the dashed frame instead of leaving a hole the size of a figure.
fn draw_image(
    canvas: &mut Canvas,
    image: &super::engine::ImageBox,
    x: f32,
    y: f32,
    column_width: f32,
    images: &HashMap<String, DecodedImage>,
    dim: f32,
) -> bool {
    let Some(decoded) = images.get(&image.path) else {
        return false;
    };
    let width = image.width.round().max(1.0) as u32;
    let height = image.height.round().max(1.0) as u32;
    let Some(pixels) = decoded.resized(width, height) else {
        return false;
    };
    // Center the image in the column.
    let offset = ((column_width - image.width) / 2.0).max(0.0);
    if dim >= 1.0 {
        canvas.draw_rgba(x + offset, y, width, height, &pixels);
        return true;
    }
    let faded: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[r, g, b, a]| [*r, *g, *b, (*a as f32 * dim) as u8])
        .collect();
    canvas.draw_rgba(x + offset, y, width, height, &faded);
    true
}
