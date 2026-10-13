//! XHTML -> blocks: structure kept, presentation discarded, bad markup survived.

mod support;

use folio_core::content::{BlockKind, Script, parse_chapter};

#[test]
fn keeps_structure_and_drops_presentation() {
    let chapter = parse_chapter(
        r#"<html><head><style>p { color: red }</style></head><body>
            <h2 id="ch2">A Heading</h2>
            <p class="fancy" style="color:#000">Plain <em>italic</em> and <strong>bold</strong>.</p>
            <script>ignored()</script>
        </body></html>"#,
    );

    assert_eq!(chapter.blocks.len(), 2);
    assert_eq!(chapter.blocks[0].kind, BlockKind::Heading(2));
    assert_eq!(chapter.blocks[0].text(), "A Heading");
    assert_eq!(chapter.blocks[0].anchors, vec!["ch2".to_string()]);

    let spans = &chapter.blocks[1].spans;
    assert_eq!(chapter.blocks[1].text(), "Plain italic and bold.");
    assert!(spans.iter().any(|s| s.text == "italic" && s.emph.italic));
    assert!(spans.iter().any(|s| s.text == "bold" && s.emph.bold));
    // CSS is gone: nothing in the model can express a color.
    assert_eq!(chapter.blocks.len(), 2);
}

#[test]
fn recovers_from_malformed_markup() {
    // Unclosed tags, bad nesting and a bare ampersand: a browser's error
    // recovery is exactly what keeps this chapter readable.
    let chapter = parse_chapter("<body><p>Smith & Co <i>drifts<p>into the next paragraph</body>");
    let text: String = chapter.blocks.iter().map(|b| b.text()).collect();
    assert!(text.contains("Smith & Co"), "entity survived: {text}");
    assert!(text.contains("into the next paragraph"));
    assert!(chapter.blocks.len() >= 2, "misnesting split the paragraphs");
}

#[test]
fn numbers_ordered_lists_and_nests_them() {
    let chapter = parse_chapter(
        "<body><ol start=\"3\"><li>third</li><li>fourth<ul><li>inner</li></ul></li></ol></body>",
    );
    let markers: Vec<String> = chapter
        .blocks
        .iter()
        .filter_map(|block| match &block.kind {
            BlockKind::ListItem { marker, depth } => Some(format!("{marker}@{depth}")),
            _ => None,
        })
        .collect();
    assert_eq!(markers, vec!["3.@1", "4.@1", "\u{2022}@2"]);
}

#[test]
fn tracks_quote_depth_and_rules() {
    let chapter = parse_chapter(
        "<body><blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote><hr/></body>",
    );
    let kinds: Vec<&BlockKind> = chapter.blocks.iter().map(|b| &b.kind).collect();
    assert_eq!(kinds[0], &BlockKind::Quote { depth: 1 });
    assert_eq!(kinds[1], &BlockKind::Quote { depth: 2 });
    assert_eq!(kinds[2], &BlockKind::Rule);
}

#[test]
fn preserves_whitespace_only_inside_pre() {
    let chapter = parse_chapter("<body><p>a   b\n  c</p><pre>one\n  two</pre></body>");
    assert_eq!(chapter.blocks[0].text(), "a b c");
    assert_eq!(chapter.blocks[1].kind, BlockKind::Pre);
    assert_eq!(chapter.blocks[1].text(), "one\n  two");
}

#[test]
fn carries_links_and_scripts_on_spans() {
    let chapter = parse_chapter(
        r##"<body><p>See <a href="ch3.xhtml#n2">note</a><sup><a href="#fn1">1</a></sup></p></body>"##,
    );
    let spans = &chapter.blocks[0].spans;
    assert_eq!(
        spans
            .iter()
            .find(|s| s.text == "note")
            .unwrap()
            .link
            .as_deref(),
        Some("ch3.xhtml#n2")
    );
    let footnote = spans.iter().find(|s| s.text == "1").unwrap();
    assert_eq!(footnote.script, Script::Super);
    assert_eq!(footnote.link.as_deref(), Some("#fn1"));
}

#[test]
fn finds_images_including_svg_wrapped_covers() {
    let chapter = parse_chapter(
        r#"<body>
            <img src="../img/plate.png" alt="A plate"/>
            <svg viewBox="0 0 10 10"><image xlink:href="cover.jpg"/></svg>
        </body>"#,
    );
    let images: Vec<(&str, &str)> = chapter
        .blocks
        .iter()
        .filter_map(|block| match &block.kind {
            BlockKind::Image { href, alt } => Some((href.as_str(), alt.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        images,
        vec![("../img/plate.png", "A plate"), ("cover.jpg", "")]
    );
}

#[test]
fn resolves_anchors_to_blocks() {
    let chapter = parse_chapter(r#"<body><p>one</p><p id="target">two</p><p>three</p></body>"#);
    assert_eq!(chapter.block_for_anchor("target"), Some(1));
    assert_eq!(chapter.block_for_anchor("missing"), None);
}
