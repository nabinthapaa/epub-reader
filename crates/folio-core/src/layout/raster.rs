//! Drawing one page into a [`Canvas`].
//!
//! Nothing here re-shapes text: it walks the placements pagination produced and
//! paints the glyphs, rules and images. That is why a theme switch or a focus
//! spotlight move costs a raster and not a layout.

use std::collections::HashMap;

use cosmic_text::{FontSystem, SwashCache};

use super::canvas::Canvas;
use super::engine::{BlockLayout, ChapterLayout};
use super::image::DecodedImage;
use super::paginate::Pagination;
use crate::content::BlockKind;
use crate::theme::{Color, Theme};

/// Where the text column sits inside the viewport, and how the page is dimmed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageFrame {
    pub viewport_width: u32,
    pub viewport_height: u32,
    /// Left edge of the text column.
    pub column_x: f32,
    /// Top edge of the content area.
    pub content_y: f32,
    /// When set, every block but this one is dimmed (focus-mode spotlight).
    pub spotlight: Option<usize>,
}

/// A hit-testable region of the rendered page.
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

/// The rendered page plus what can be clicked on it.
pub struct RenderedPage {
    pub canvas: Canvas,
    pub links: Vec<LinkRegion>,
}

/// Render page `page_index` of `layout`.
#[allow(clippy::too_many_arguments)]
pub fn render_page(
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    layout: &ChapterLayout,
    pagination: &Pagination,
    page_index: usize,
    frame: PageFrame,
    theme: &Theme,
    images: &HashMap<String, DecodedImage>,
) -> RenderedPage {
    let mut canvas = Canvas::new(frame.viewport_width, frame.viewport_height, theme.page);
    let mut links = Vec::new();
    let Some(page) = pagination.pages.get(page_index) else {
        return RenderedPage { canvas, links };
    };

    for placement in &page.placements {
        let Some(block) = layout.blocks.get(placement.block) else {
            continue;
        };
        let dim = match frame.spotlight {
            Some(active) if active != placement.block => theme.dim_factor,
            _ => 1.0,
        };
        let x = frame.column_x + block.indent;
        let first_top = block
            .lines
            .get(placement.line_from)
            .map(|line| line.top)
            .unwrap_or(0.0);
        let y = frame.content_y + placement.y;

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
                // The quote bar spans only the lines on this page.
                let height: f32 = block.lines[placement.line_from..placement.line_to]
                    .iter()
                    .map(|line| line.height)
                    .sum();
                let bar_width = (block.indent * 0.12).max(2.0);
                canvas.fill_rect(
                    x - block.indent * 0.45,
                    y,
                    bar_width,
                    height,
                    theme.accent.fade(dim * 0.8),
                );
            }
            _ => {}
        }

        if let Some(image) = &block.image {
            if image.available {
                draw_image(&mut canvas, image, x, y, layout.column_width, images, dim);
                continue;
            }
            // Unavailable: frame the area, then fall through to draw alt text.
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
            placement.line_from,
            placement.line_to,
            x,
            y - first_top,
            theme,
            dim,
            &mut links,
        );
    }

    RenderedPage { canvas, links }
}

#[allow(clippy::too_many_arguments)]
fn draw_block_text(
    canvas: &mut Canvas,
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    block: &BlockLayout,
    line_from: usize,
    line_to: usize,
    x: f32,
    y_origin: f32,
    theme: &Theme,
    dim: f32,
    links: &mut Vec<LinkRegion>,
) {
    let Some(buffer) = block.buffer.as_ref() else {
        return;
    };
    for (run_index, run) in buffer.layout_runs().enumerate() {
        if run_index < line_from || run_index >= line_to {
            continue;
        }
        let baseline = y_origin + run.line_y;
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
                    y_origin + run.line_top,
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

fn draw_image(
    canvas: &mut Canvas,
    image: &super::engine::ImageBox,
    x: f32,
    y: f32,
    column_width: f32,
    images: &HashMap<String, DecodedImage>,
    dim: f32,
) {
    let Some(decoded) = images.get(&image.path) else {
        return;
    };
    let width = image.width.round().max(1.0) as u32;
    let height = image.height.round().max(1.0) as u32;
    let Some(pixels) = decoded.resized(width, height) else {
        return;
    };
    // Center the image in the column.
    let offset = ((column_width - image.width) / 2.0).max(0.0);
    if dim >= 1.0 {
        canvas.draw_rgba(x + offset, y, width, height, &pixels);
        return;
    }
    let faded: Vec<u8> = pixels
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2], (p[3] as f32 * dim) as u8])
        .collect();
    canvas.draw_rgba(x + offset, y, width, height, &faded);
}
