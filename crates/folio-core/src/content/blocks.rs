//! The content model: what a chapter looks like once the publisher's markup and
//! CSS have been discarded and only structure remains.
//!
//! Everything downstream (layout, pagination, raster, search) works on this and
//! never sees XHTML.

/// Inline emphasis carried by a run of text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Emphasis {
    pub italic: bool,
    pub bold: bool,
}

/// Vertical position of a run relative to the baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// A run of text sharing one set of inline attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub emph: Emphasis,
    /// Rendered in the monospace family (`code`, `kbd`, `samp`, `tt`).
    pub mono: bool,
    pub script: Script,
    /// Raw, still-unresolved href of the enclosing `<a>`, if any.
    pub link: Option<String>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            emph: Emphasis::default(),
            mono: false,
            script: Script::Normal,
            link: None,
        }
    }
}

/// What kind of block this is; drives the type scale and spacing.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    /// `h1`..`h6`, clamped to 1..=6.
    Heading(u8),
    Para,
    /// `blockquote`, nested by `depth` (1-based).
    Quote {
        depth: u8,
    },
    /// A list item carrying its already-rendered marker ("3." or "\u{2022}").
    ListItem {
        marker: String,
        depth: u8,
    },
    /// Preformatted text: whitespace is preserved and lines are not merged.
    Pre,
    /// `hr`
    Rule,
    /// An image, resolved against the chapter's own path at layout time.
    Image {
        href: String,
        alt: String,
    },
    /// `figcaption`, typeset smaller and centered under its figure.
    Caption,
}

/// One laid-out-able unit of content.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
    /// Fragment ids (`id=` attributes) that point at this block, used to
    /// resolve internal links like `chapter.xhtml#footnote-3`.
    pub anchors: Vec<String>,
}

impl Block {
    pub fn new(kind: BlockKind, spans: Vec<Span>) -> Self {
        Self {
            kind,
            spans,
            anchors: Vec::new(),
        }
    }

    /// The block's text with inline attributes flattened away; used for search
    /// and for the plain-text fallbacks.
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        match &self.kind {
            BlockKind::Rule | BlockKind::Image { .. } => false,
            _ => self.spans.iter().all(|s| s.text.trim().is_empty()),
        }
    }
}

/// A parsed chapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chapter {
    pub blocks: Vec<Block>,
}

impl Chapter {
    /// Index of the block carrying `anchor`, for internal-link navigation.
    pub fn block_for_anchor(&self, anchor: &str) -> Option<usize> {
        self.blocks
            .iter()
            .position(|b| b.anchors.iter().any(|a| a == anchor))
    }

    /// Whole-chapter plain text, one block per line, for the search index.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            out.push_str(&block.text());
            out.push('\n');
        }
        out
    }
}
