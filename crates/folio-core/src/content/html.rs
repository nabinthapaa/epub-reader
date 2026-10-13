//! XHTML -> [`Chapter`] conversion.
//!
//! Parsed with html5ever's full tree construction, so malformed markup (stray
//! `&`, unclosed tags, bad nesting) is recovered exactly as a browser would and
//! a chapter never fails to open. Stylesheets and `style` attributes are
//! discarded by design: only structure survives, and our theme and type scale
//! decide how it looks.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use super::blocks::{Block, BlockKind, Chapter, Emphasis, Script, Span};

/// Parse one chapter document. Never fails.
pub fn parse_chapter(xhtml: &str) -> Chapter {
    let dom = html5ever::parse_document(RcDom::default(), ParseOpts::default()).one(xhtml);
    let mut builder = Builder::default();
    builder.walk(&dom.document);
    builder.finish()
}

/// Inline attributes currently in scope.
#[derive(Clone, Debug, Default)]
struct Inline {
    emph: Emphasis,
    mono: bool,
    script: Script,
    link: Option<String>,
}

#[derive(Debug)]
struct ListCtx {
    ordered: bool,
    counter: usize,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    kind: Option<BlockKind>,
    pending_anchors: Vec<String>,
    inline: Inline,
    quote_depth: u8,
    lists: Vec<ListCtx>,
    pre_depth: usize,
}

impl Builder {
    fn finish(mut self) -> Chapter {
        self.flush();
        // Anchors that pointed at trailing empty markup still need a home.
        if !self.pending_anchors.is_empty()
            && let Some(last) = self.blocks.last_mut()
        {
            last.anchors.append(&mut self.pending_anchors);
        }
        Chapter {
            blocks: self.blocks,
        }
    }

    /// The kind a block started right now should have.
    fn default_kind(&self) -> BlockKind {
        if self.quote_depth > 0 {
            BlockKind::Quote {
                depth: self.quote_depth,
            }
        } else {
            BlockKind::Para
        }
    }

    /// Close the block under construction, if it holds anything.
    fn flush(&mut self) {
        let kind = self.kind.take();
        let spans = std::mem::take(&mut self.spans);
        let kind = kind.unwrap_or_else(|| self.default_kind());
        let mut block = Block::new(kind, spans);
        if block.is_empty() {
            return;
        }
        trim_block(&mut block);
        block.anchors = std::mem::take(&mut self.pending_anchors);
        self.blocks.push(block);
    }

    /// Emit a block that has no inline content of its own.
    fn push_standalone(&mut self, kind: BlockKind) {
        self.flush();
        let mut block = Block::new(kind, Vec::new());
        block.anchors = std::mem::take(&mut self.pending_anchors);
        self.blocks.push(block);
    }

    fn push_text(&mut self, raw: &str) {
        if self.pre_depth > 0 {
            self.push_span(raw.to_string());
            return;
        }
        let at_block_start = self.spans.iter().all(|s| s.text.is_empty());
        let collapsed = collapse_whitespace(raw, at_block_start);
        if collapsed.is_empty() {
            return;
        }
        self.push_span(collapsed);
    }

    fn push_span(&mut self, text: String) {
        // Merge into the previous span when the attributes match, so a
        // paragraph of plain text is one span rather than dozens.
        if let Some(last) = self.spans.last_mut()
            && last.emph == self.inline.emph
            && last.mono == self.inline.mono
            && last.script == self.inline.script
            && last.link == self.inline.link
        {
            last.text.push_str(&text);
            return;
        }
        self.spans.push(Span {
            text,
            emph: self.inline.emph,
            mono: self.inline.mono,
            script: self.inline.script,
            link: self.inline.link.clone(),
        });
    }

    fn walk(&mut self, node: &Handle) {
        match &node.data {
            NodeData::Text { contents } => self.push_text(&contents.borrow()),
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref().to_ascii_lowercase();
                let attrs = attrs.borrow();
                let attrs = attrs.as_slice();
                if let Some(id) = attr(attrs, "id") {
                    self.pending_anchors.push(id);
                }
                self.element(&tag, attrs, node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: &Handle) {
        for child in node.children.borrow().iter() {
            self.walk(child);
        }
    }

    fn element(&mut self, tag: &str, attrs: &[html5ever::Attribute], node: &Handle) {
        match tag {
            // Never readable content.
            "head" | "script" | "style" | "template" | "link" | "meta" | "title" | "base"
            | "noscript" | "iframe" | "object" | "audio" | "video" => {}

            // Images, including the SVG wrappers EPUB covers like to use.
            "img" => {
                let href = attr(attrs, "src").unwrap_or_default();
                let alt = attr(attrs, "alt").unwrap_or_default();
                if !href.is_empty() {
                    self.push_standalone(BlockKind::Image { href, alt });
                }
            }
            "svg" => {
                if let Some((href, alt)) = svg_image(node) {
                    self.push_standalone(BlockKind::Image { href, alt });
                }
            }
            "image" => {
                let href = attr(attrs, "href")
                    .or_else(|| attr(attrs, "xlink:href"))
                    .unwrap_or_default();
                if !href.is_empty() {
                    self.push_standalone(BlockKind::Image {
                        href,
                        alt: String::new(),
                    });
                }
            }

            "hr" => self.push_standalone(BlockKind::Rule),
            "br" => self.push_span("\n".into()),

            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                let level = tag.as_bytes()[1] - b'0';
                self.kind = Some(BlockKind::Heading(level.clamp(1, 6)));
                self.walk_children(node);
                self.flush();
            }

            "blockquote" => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_add(1);
                self.walk_children(node);
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }

            "ul" | "ol" => {
                self.flush();
                self.lists.push(ListCtx {
                    ordered: tag == "ol",
                    counter: attr(attrs, "start")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                        .map_or(0, |n| n.saturating_sub(1)),
                });
                self.walk_children(node);
                self.flush();
                self.lists.pop();
            }

            "li" => {
                self.flush();
                let depth = self.lists.len().clamp(1, u8::MAX as usize) as u8;
                let marker = match self.lists.last_mut() {
                    Some(ctx) if ctx.ordered => {
                        ctx.counter += 1;
                        format!("{}.", ctx.counter)
                    }
                    _ => "\u{2022}".to_string(),
                };
                self.kind = Some(BlockKind::ListItem { marker, depth });
                self.walk_children(node);
                self.flush();
            }

            "pre" => {
                self.flush();
                self.pre_depth += 1;
                self.kind = Some(BlockKind::Pre);
                self.walk_children(node);
                self.flush();
                self.pre_depth -= 1;
            }

            "figcaption" => {
                self.flush();
                self.kind = Some(BlockKind::Caption);
                self.walk_children(node);
                self.flush();
            }

            // Inline emphasis.
            "em" | "i" | "cite" | "dfn" | "var" => self.with_inline(node, |i| i.emph.italic = true),
            "strong" | "b" => self.with_inline(node, |i| i.emph.bold = true),
            "code" | "kbd" | "samp" | "tt" => self.with_inline(node, |i| i.mono = true),
            "sup" => self.with_inline(node, |i| i.script = Script::Super),
            "sub" => self.with_inline(node, |i| i.script = Script::Sub),
            "a" => {
                let href = attr(attrs, "href").filter(|h| !h.trim().is_empty());
                self.with_inline(node, |i| i.link = href);
            }

            // Block containers: start a fresh block, then pass through.
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "aside"
            | "figure" | "nav" | "dl" | "dt" | "dd" | "table" | "thead" | "tbody" | "tfoot"
            | "tr" | "td" | "th" | "caption" | "body" | "html" => {
                self.flush();
                self.walk_children(node);
                self.flush();
            }

            // Everything else (span, small, abbr, q, u, s, ruby, ...) is
            // transparent: keep the text, drop the tag.
            _ => self.walk_children(node),
        }
    }

    fn with_inline(&mut self, node: &Handle, edit: impl FnOnce(&mut Inline)) {
        let saved = self.inline.clone();
        edit(&mut self.inline);
        self.walk_children(node);
        self.inline = saved;
    }
}

fn attr(attrs: &[html5ever::Attribute], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|a| str::eq_ignore_ascii_case(&a.name.local, name))
        .map(|a| a.value.to_string())
}

/// Find the first raster image referenced inside an `<svg>` wrapper.
fn svg_image(node: &Handle) -> Option<(String, String)> {
    for child in node.children.borrow().iter() {
        if let NodeData::Element { name, attrs, .. } = &child.data {
            let tag = name.local.as_ref().to_ascii_lowercase();
            if tag == "image" || tag == "img" {
                let attrs = attrs.borrow();
                let attrs = attrs.as_slice();
                let href = attr(attrs, "href")
                    .or_else(|| attr(attrs, "xlink:href"))
                    .or_else(|| attr(attrs, "src"))?;
                if !href.trim().is_empty() {
                    return Some((href, attr(attrs, "alt").unwrap_or_default()));
                }
            }
        }
        if let Some(found) = svg_image(child) {
            return Some(found);
        }
    }
    None
}

/// Collapse runs of whitespace into single spaces, dropping the leading space
/// when nothing has been emitted into the block yet.
fn collapse_whitespace(raw: &str, at_block_start: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !(out.is_empty() && at_block_start) {
            out.push(' ');
        }
        pending_space = false;
        out.push(ch);
    }
    if pending_space && !out.is_empty() {
        out.push(' ');
    }
    out
}

/// Drop leading/trailing whitespace at the block's edges.
fn trim_block(block: &mut Block) {
    if matches!(block.kind, BlockKind::Pre) {
        return;
    }
    if let Some(first) = block.spans.first_mut() {
        let trimmed = first.text.trim_start().to_string();
        first.text = trimmed;
    }
    if let Some(last) = block.spans.last_mut() {
        let trimmed = last.text.trim_end().to_string();
        last.text = trimmed;
    }
    block.spans.retain(|s| !s.text.is_empty());
}
