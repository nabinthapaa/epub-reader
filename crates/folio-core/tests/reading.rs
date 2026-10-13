//! The reading session end to end: navigation, zoom anchoring, focus mode,
//! links, search — plus the rbook adapter against a real generated EPUB.

mod support;

use folio_core::book::ports::{Target, TocItem};
use folio_core::reader::{Command, Reader};
use folio_core::state::{Position, Settings};
use folio_core::theme::{Theme, ThemePreset};
use support::FakeBook;

const VIEWPORT: (u32, u32) = (900, 600);

fn reader_with(chapters: Vec<(&str, &str)>) -> Reader {
    Reader::with_book(
        Box::new(FakeBook::new(chapters)),
        "/tmp/test.epub".into(),
        &Settings::default(),
        ThemePreset::Light,
        VIEWPORT,
        None,
    )
    .expect("reader opens")
}

#[test]
fn turning_past_the_last_page_moves_to_the_next_chapter() {
    let long = support::paragraphs(30);
    let mut reader = reader_with(vec![
        ("one.xhtml", long.as_str()),
        ("two.xhtml", "<p>second chapter</p>"),
    ]);

    let view = reader.apply(Command::Refresh).unwrap();
    assert_eq!(view.chapter, 0);
    assert!(view.page_count > 1, "fixture spans pages");

    for _ in 0..view.page_count {
        reader.apply(Command::Page(1)).unwrap();
    }
    let view = reader.apply(Command::Refresh).unwrap();
    assert_eq!(view.chapter, 1, "page turns cross the chapter boundary");
    assert_eq!(view.page, 0);

    // And back again.
    let view = reader.apply(Command::Page(-1)).unwrap();
    assert_eq!(view.chapter, 0);
    assert_eq!(view.page, view.page_count - 1, "lands on the last page");
}

#[test]
fn the_first_page_stays_put_at_the_start_of_the_book() {
    let mut reader = reader_with(vec![("one.xhtml", "<p>only</p>")]);
    let view = reader.apply(Command::Page(-1)).unwrap();
    assert_eq!((view.chapter, view.page), (0, 0));
}

#[test]
fn zooming_keeps_the_reader_in_the_same_place() {
    let long = support::paragraphs(60);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);

    for _ in 0..3 {
        reader.apply(Command::Page(1)).unwrap();
    }
    let before = reader.apply(Command::Refresh).unwrap();
    let anchor = before.position;

    let zoomed = reader.apply(Command::Zoom(2)).unwrap();
    assert!(zoomed.font_px > before.font_px, "font actually grew");
    assert_eq!(
        zoomed.position.block, anchor.block,
        "the block at the top of the page survives a zoom"
    );
    assert!(
        zoomed.page_count > before.page_count,
        "bigger text means more pages"
    );

    let back = reader.apply(Command::ZoomReset).unwrap();
    assert_eq!(back.font_px, before.font_px);
    assert_eq!(back.position.block, anchor.block);
}

#[test]
fn focus_mode_narrows_the_column_without_losing_the_place() {
    let long = support::paragraphs(40);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);
    for _ in 0..2 {
        reader.apply(Command::Page(1)).unwrap();
    }
    let before = reader.apply(Command::Refresh).unwrap();

    let focused = reader.apply(Command::SetFocus(true)).unwrap();
    assert!(focused.focus);
    assert_eq!(focused.position.block, before.position.block);
    assert!(
        focused.page_count >= before.page_count,
        "a narrower measure cannot produce fewer pages"
    );

    let unfocused = reader.apply(Command::SetFocus(false)).unwrap();
    assert!(!unfocused.focus);
    assert_eq!(unfocused.position.block, before.position.block);
}

#[test]
fn following_a_link_jumps_and_back_returns() {
    // The target sits deep in the second chapter, so landing on it has to be
    // the anchor's doing and not simply "the chapter fits on one page".
    let second = format!(
        "{}<p id=\"target\">the note itself</p>{}",
        support::paragraphs(40),
        support::paragraphs(10)
    );
    let mut reader = reader_with(vec![
        (
            "one.xhtml",
            r#"<p>see <a href="two.xhtml#target">the note</a></p>"#,
        ),
        ("two.xhtml", second.as_str()),
    ]);

    let start = reader.apply(Command::Refresh).unwrap();
    assert_eq!(start.links.len(), 1, "the link is hit-testable");
    assert!(!start.can_go_back);

    let jumped = reader
        .apply(Command::FollowLink("two.xhtml#target".into()))
        .unwrap();
    assert_eq!(jumped.chapter, 1);
    assert!(jumped.page > 0, "the anchor is past the first page");
    assert!(jumped.can_go_back);

    // The page shown is the one that holds the anchored block.
    let anchor_block = 40;
    let direct = reader
        .apply(Command::GoTo(Position {
            chapter: 1,
            block: anchor_block,
            char_offset: 0,
        }))
        .unwrap();
    assert_eq!(direct.page, jumped.page, "link and position agree");

    // Back unwinds the jump that was followed, not the direct navigation.
    let returned = reader.apply(Command::Back).unwrap();
    assert_eq!(returned.chapter, 0);
}

#[test]
fn external_links_are_left_alone() {
    let mut reader = reader_with(vec![(
        "one.xhtml",
        r#"<p><a href="https://example.org">out</a></p>"#,
    )]);
    let view = reader
        .apply(Command::FollowLink("https://example.org".into()))
        .unwrap();
    assert_eq!(view.chapter, 0, "external links do not move the cursor");
    assert!(!view.can_go_back);
}

#[test]
fn search_finds_text_and_the_hit_navigates_to_it() {
    let mut reader = reader_with(vec![
        ("one.xhtml", "<p>nothing here</p>"),
        ("two.xhtml", "<p>filler</p><p>a peculiar apparatus hums</p>"),
    ]);

    let hits = reader.search("APPARATUS", 10);
    assert_eq!(hits.len(), 1, "search is case-insensitive");
    assert_eq!(hits[0].position.chapter, 1);
    assert_eq!(hits[0].position.block, 1);
    assert!(hits[0].snippet.contains("apparatus"));

    let view = reader.apply(Command::GoTo(hits[0].position)).unwrap();
    assert_eq!(view.chapter, 1);
    assert!(
        reader.search("", 10).is_empty(),
        "empty query finds nothing"
    );
}

#[test]
fn contents_entries_jump_to_their_anchor() {
    let body = format!(
        "{}<h2 id=\"part2\">Part Two</h2>{}",
        support::paragraphs(40),
        support::paragraphs(10)
    );
    let book = FakeBook::new(vec![("one.xhtml", body.as_str())]).with_toc(vec![
        TocItem {
            label: "Front".into(),
            depth: 0,
            target: Some(Target {
                chapter: 0,
                anchor: None,
            }),
        },
        TocItem {
            label: "Part Two".into(),
            depth: 0,
            target: Some(Target {
                chapter: 0,
                anchor: Some("part2".into()),
            }),
        },
    ]);
    let mut reader = Reader::with_book(
        Box::new(book),
        "/tmp/test.epub".into(),
        &Settings::default(),
        ThemePreset::Light,
        VIEWPORT,
        None,
    )
    .unwrap();

    let start = reader.apply(Command::Refresh).unwrap();
    assert_eq!(start.toc_index, Some(0), "starts inside the first entry");

    let view = reader.apply(Command::GoToToc(1)).unwrap();
    assert!(view.page > 0, "the heading is past the first page");
    assert_eq!(
        view.toc_index,
        Some(1),
        "the contents panel highlights the entry we are inside"
    );

    let back_to_front = reader.apply(Command::GoToToc(0)).unwrap();
    assert_eq!(back_to_front.page, 0);
    assert_eq!(back_to_front.toc_index, Some(0));
}

#[test]
fn themes_change_pixels_without_moving_the_text() {
    let mut reader = reader_with(vec![("one.xhtml", "<p>some words to paint</p>")]);
    let light = reader
        .apply(Command::SetTheme(Theme::new(ThemePreset::Light)))
        .unwrap();
    let dark = reader
        .apply(Command::SetTheme(Theme::new(ThemePreset::Dark)))
        .unwrap();

    assert_eq!(light.canvas.rgba.len(), dark.canvas.rgba.len());
    assert_ne!(light.canvas.rgba, dark.canvas.rgba, "the page is repainted");
    assert_eq!(&dark.canvas.rgba[..4], &[0x16, 0x17, 0x19, 0xFF]);
    assert_eq!(
        light.position, dark.position,
        "a theme switch never moves the reader"
    );
}

#[test]
fn opens_a_real_epub_through_the_rbook_adapter() {
    let path = support::write_epub(
        &[
            (
                "one.xhtml",
                r#"<h1>One</h1><p>see <a href="two.xhtml#x">it</a></p>"#,
            ),
            ("two.xhtml", r#"<p id="x">there</p>"#),
        ],
        "adapter.epub",
    );

    let mut reader = Reader::open(
        &path,
        &Settings::default(),
        ThemePreset::Light,
        VIEWPORT,
        None,
    )
    .expect("the generated EPUB opens");

    assert_eq!(reader.meta().title, "Test Book");
    assert_eq!(reader.meta().authors, vec!["A. Tester".to_string()]);
    assert_eq!(reader.chapter_count(), 2);
    assert_eq!(reader.toc().len(), 2);

    let view = reader
        .apply(Command::FollowLink("two.xhtml#x".into()))
        .unwrap();
    assert_eq!(view.chapter, 1, "relative links resolve inside the archive");
    assert_eq!(
        reader
            .apply(Command::GoTo(Position::chapter_start(0)))
            .unwrap()
            .chapter,
        0
    );
}
