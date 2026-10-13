//! Chapter layout: blocks in, positioned lines out.
//!
//! Each block becomes one cosmic-text buffer shaped at the column width, so the
//! expensive work happens once per chapter per type setting. Pagination and
//! raster then only slice and draw.

use std::collections::HashMap;

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap,
};

use super::type_settings::{Alignment, TypeSettings};
use crate::content::{Block, BlockKind, Chapter, Script};

/// One visual line produced by shaping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBox {
    /// Index of the layout run inside the block's buffer.
    pub run: usize,
    /// Offset of the line's top from the block's top, in px.
    pub top: f32,
    pub height: f32,
    pub width: f32,
}

/// How an image block was sized into the column.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageBox {
    /// Normalized archive path, resolved against the chapter.
    pub path: String,
    pub width: f32,
    pub height: f32,
    pub alt: String,
    /// False when the bytes are missing or undecodable; the alt text is then
    /// drawn inside a dashed frame instead.
    pub available: bool,
}

/// What a span means to the rasterizer: which color it takes, and where it
/// points if it is a link.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpanRole {
    pub link: Option<String>,
    pub muted: bool,
}

/// A laid-out block.
pub struct BlockLayout {
    /// Index of the source block in the chapter.
    pub block: usize,
    pub kind: BlockKind,
    pub buffer: Option<Buffer>,
    pub image: Option<ImageBox>,
    pub lines: Vec<LineBox>,
    /// Left offset inside the column (quotes, list items).
    pub indent: f32,
    pub space_before: f32,
    pub space_after: f32,
    /// Total height of the block's lines.
    pub height: f32,
    /// Indexed by the `metadata` carried on each glyph.
    pub roles: Vec<SpanRole>,
}

/// A chapter laid out at one specific width, height and type setting.
pub struct ChapterLayout {
    pub chapter: usize,
    pub column_width: f32,
    pub blocks: Vec<BlockLayout>,
    /// Fragment id -> index into `blocks`.
    pub anchors: HashMap<String, usize>,
    /// Plain text per source block, kept for search snippets.
    pub block_text: Vec<String>,
}

impl ChapterLayout {
    /// Index into `blocks` for a source block index, or the nearest following
    /// block when that exact one produced no lines.
    pub fn position_of_block(&self, block: usize) -> Option<usize> {
        self.blocks
            .iter()
            .position(|b| b.block == block)
            .or_else(|| self.blocks.iter().position(|b| b.block > block))
    }
}

/// Type scale relative to the base font size.
fn scale_for(kind: &BlockKind) -> f32 {
    match kind {
        BlockKind::Heading(1) => 1.9,
        BlockKind::Heading(2) => 1.55,
        BlockKind::Heading(3) => 1.3,
        BlockKind::Heading(4) => 1.15,
        BlockKind::Heading(_) => 1.0,
        BlockKind::Pre => 0.9,
        BlockKind::Caption => 0.85,
        _ => 1.0,
    }
}

/// Vertical rhythm, in multiples of the base font size.
fn spacing_for(kind: &BlockKind) -> (f32, f32) {
    match kind {
        BlockKind::Heading(1 | 2) => (1.4, 0.6),
        BlockKind::Heading(_) => (1.1, 0.45),
        BlockKind::Para => (0.0, 0.7),
        BlockKind::Quote { .. } => (0.6, 0.6),
        BlockKind::ListItem { .. } => (0.0, 0.3),
        BlockKind::Pre => (0.8, 0.8),
        BlockKind::Rule => (1.2, 1.2),
        BlockKind::Image { .. } => (0.9, 0.5),
        BlockKind::Caption => (0.2, 0.9),
    }
}

/// Lays chapters out. Owns the font system, so it lives on the worker thread.
pub struct Layouter {
    pub fonts: FontSystem,
}

/// Intrinsic size of a resolved image resource.
pub struct ImageInfo {
    /// Normalized archive path the rasterizer will look the pixels up by.
    pub path: String,
    pub width: u32,
    pub height: u32,
}

impl Layouter {
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
        }
    }

    /// Lay out a whole chapter.
    ///
    /// `column_width` is the text column in px and `max_block_height` the
    /// usable page height, used to scale an oversized image down so it can
    /// occupy a page of its own instead of never fitting.
    ///
    /// `resolve_image` maps an href to a decoded resource; returning `None`
    /// means the resource is missing or unsupported, and the alt text is framed
    /// in its place.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_chapter(
        &mut self,
        chapter_index: usize,
        chapter: &Chapter,
        settings: &TypeSettings,
        column_width: f32,
        max_block_height: f32,
        body_family: &str,
        mono_family: &str,
        mut resolve_image: impl FnMut(&str) -> Option<ImageInfo>,
    ) -> ChapterLayout {
        let mut blocks = Vec::with_capacity(chapter.blocks.len());
        let mut anchors = HashMap::new();
        let mut block_text = Vec::with_capacity(chapter.blocks.len());

        for (index, block) in chapter.blocks.iter().enumerate() {
            block_text.push(block.text());
            let laid = self.layout_block(
                index,
                block,
                settings,
                column_width,
                max_block_height,
                body_family,
                mono_family,
                &mut resolve_image,
            );
            for anchor in &block.anchors {
                anchors.insert(anchor.clone(), blocks.len());
            }
            blocks.push(laid);
        }

        ChapterLayout {
            chapter: chapter_index,
            column_width,
            blocks,
            anchors,
            block_text,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_block(
        &mut self,
        index: usize,
        block: &Block,
        settings: &TypeSettings,
        column_width: f32,
        max_block_height: f32,
        body_family: &str,
        mono_family: &str,
        resolve_image: &mut impl FnMut(&str) -> Option<ImageInfo>,
    ) -> BlockLayout {
        let (before, after) = spacing_for(&block.kind);
        let indent = match &block.kind {
            BlockKind::Quote { depth } => settings.font_px * 1.4 * f32::from(*depth),
            BlockKind::ListItem { depth, .. } => settings.font_px * 1.4 * f32::from(*depth),
            _ => 0.0,
        };
        let mut layout = BlockLayout {
            block: index,
            kind: block.kind.clone(),
            buffer: None,
            image: None,
            lines: Vec::new(),
            indent,
            space_before: before * settings.font_px,
            space_after: after * settings.font_px,
            height: 0.0,
            roles: Vec::new(),
        };
        let available_width = (column_width - indent).max(1.0);

        match &block.kind {
            BlockKind::Rule => {
                let height = (settings.font_px * 0.1).max(1.0);
                layout.lines.push(LineBox {
                    run: 0,
                    top: 0.0,
                    height,
                    width: column_width,
                });
                layout.height = height;
                return layout;
            }
            BlockKind::Image { href, alt } => {
                match resolve_image(href) {
                    Some(info) => {
                        let intrinsic_w = info.width.max(1) as f32;
                        let intrinsic_h = info.height.max(1) as f32;
                        let mut width = available_width.min(intrinsic_w);
                        let mut height = width * intrinsic_h / intrinsic_w;
                        // Scale an oversized figure down so it fits one page.
                        let ceiling = (max_block_height - settings.font_px).max(settings.font_px);
                        if height > ceiling {
                            let factor = ceiling / height;
                            height = ceiling;
                            width *= factor;
                        }
                        layout.lines.push(LineBox {
                            run: 0,
                            top: 0.0,
                            height,
                            width,
                        });
                        layout.height = height;
                        layout.image = Some(ImageBox {
                            path: info.path,
                            width,
                            height,
                            alt: alt.clone(),
                            available: true,
                        });
                        return layout;
                    }
                    None => {
                        // Fall through to the text path so the alt text is laid
                        // out; the frame is drawn around it at raster time.
                        let label = if alt.trim().is_empty() {
                            "[image]".to_string()
                        } else {
                            alt.clone()
                        };
                        let font_px = settings.font_px * 0.85;
                        let (buffer, lines, height) = self.shape(
                            &[(
                                label,
                                Attrs::new().family(Family::Name(body_family)).metadata(0),
                            )],
                            Metrics::new(font_px, font_px * settings.line_height),
                            available_width,
                            Some(Align::Center),
                            body_family,
                        );
                        layout.roles = vec![SpanRole {
                            link: None,
                            muted: true,
                        }];
                        layout.lines = lines;
                        layout.height = height;
                        layout.buffer = Some(buffer);
                        layout.image = Some(ImageBox {
                            path: href.clone(),
                            width: available_width,
                            height,
                            alt: alt.clone(),
                            available: false,
                        });
                        return layout;
                    }
                }
            }
            _ => {}
        }

        // Text blocks: build the rich-text runs, tagging each with the index of
        // its role so the rasterizer can color it without re-shaping.
        let scale = scale_for(&block.kind);
        let font_px = settings.font_px * scale;
        let metrics = Metrics::new(font_px, font_px * settings.line_height);
        let mono_block = matches!(block.kind, BlockKind::Pre);
        let heading = matches!(block.kind, BlockKind::Heading(_));

        let mut runs: Vec<(String, Attrs<'_>)> = Vec::new();
        let mut roles: Vec<SpanRole> = Vec::new();

        if let BlockKind::ListItem { marker, .. } = &block.kind {
            roles.push(SpanRole {
                link: None,
                muted: true,
            });
            runs.push((
                format!("{marker}  "),
                Attrs::new().family(Family::Name(body_family)).metadata(0),
            ));
        }

        for span in &block.spans {
            let role_index = roles.len();
            roles.push(SpanRole {
                link: span.link.clone(),
                muted: matches!(block.kind, BlockKind::Caption),
            });
            let family = if span.mono || mono_block {
                Family::Name(mono_family)
            } else {
                Family::Name(body_family)
            };
            let mut attrs = Attrs::new().family(family).metadata(role_index);
            if span.emph.italic {
                attrs = attrs.style(Style::Italic);
            }
            if span.emph.bold || heading {
                attrs = attrs.weight(Weight::BOLD);
            }
            if span.script != Script::Normal {
                let small = font_px * 0.72;
                attrs = attrs.metrics(Metrics::new(small, font_px * settings.line_height));
            }
            runs.push((span.text.clone(), attrs));
        }

        if runs.is_empty() {
            return layout;
        }

        let align = match (&block.kind, settings.alignment) {
            (BlockKind::Caption, _) => Some(Align::Center),
            (BlockKind::Heading(_) | BlockKind::Pre, _) => Some(Align::Left),
            (_, Alignment::Justified) => Some(Align::Justified),
            (_, Alignment::Left) => Some(Align::Left),
        };

        let (buffer, lines, height) =
            self.shape(&runs, metrics, available_width, align, body_family);
        layout.lines = lines;
        layout.height = height;
        layout.buffer = Some(buffer);
        layout.roles = roles;
        layout
    }

    /// Shape one block's runs into a buffer and collect its line boxes.
    fn shape(
        &mut self,
        runs: &[(String, Attrs<'_>)],
        metrics: Metrics,
        width: f32,
        align: Option<Align>,
        default_family: &str,
    ) -> (Buffer, Vec<LineBox>, f32) {
        let default_attrs = Attrs::new().family(Family::Name(default_family));
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        {
            let mut borrowed = buffer.borrow_with(&mut self.fonts);
            borrowed.set_wrap(Wrap::WordOrGlyph);
            borrowed.set_size(Some(width), None);
            borrowed.set_rich_text(
                runs.iter()
                    .map(|(text, attrs)| (text.as_str(), attrs.clone())),
                &default_attrs,
                Shaping::Advanced,
                align,
            );
            borrowed.shape_until_scroll(false);
        }
        let mut lines = Vec::new();
        let mut height = 0.0_f32;
        for (run_index, run) in buffer.layout_runs().enumerate() {
            lines.push(LineBox {
                run: run_index,
                top: run.line_top,
                height: run.line_height,
                width: run.line_w,
            });
            height = height.max(run.line_top + run.line_height);
        }
        (buffer, lines, height)
    }
}

impl Default for Layouter {
    fn default() -> Self {
        Self::new()
    }
}
