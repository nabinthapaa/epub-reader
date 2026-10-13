//! The reading session end to end: navigation, zoom anchoring, focus mode,
//! links, search — plus the rbook adapter against a real generated EPUB.

mod support;

use folio_core::book::ports::{Target, TocItem};
use folio_core::reader::{Command, Reader, Transition};
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
fn scrolling_is_continuous_until_the_chapter_ends() {
    let long = support::paragraphs(30);
    let mut reader = reader_with(vec![
        ("one.xhtml", long.as_str()),
        ("two.xhtml", "<p>second chapter</p>"),
    ]);

    let start = reader.apply(Command::Refresh).unwrap();
    assert_eq!(start.chapter, 0);
    assert_eq!(start.scroll, 0.0);
    assert!(
        start.chapter_height > start.viewport_height,
        "fixture should be scrollable"
    );

    // Small scrolls move within the chapter and do not animate anything.
    let scrolled = reader.apply(Command::Scroll(120.0)).unwrap();
    assert_eq!(scrolled.scroll, 120.0);
    assert_eq!(scrolled.chapter, 0);
    assert!(scrolled.transition.is_none(), "no seam inside a chapter");

    // Scrolling to the very bottom stays in the chapter...
    let bottom = reader.apply(Command::Scroll(1_000_000.0)).unwrap();
    assert_eq!(
        bottom.chapter, 0,
        "the first overscroll only reaches the end"
    );
    assert!(bottom.transition.is_none());
    assert!(bottom.scroll > 0.0);

    // ...and only the next scroll crosses over, with a transition to animate.
    let next = reader.apply(Command::Scroll(80.0)).unwrap();
    assert_eq!(
        next.chapter, 1,
        "scrolling past the end enters the next chapter"
    );
    assert_eq!(next.scroll, 0.0);
    assert_eq!(next.transition, Some(Transition::Forward));
    assert!(next.band.is_some(), "a new chapter always brings pixels");

    // Scrolling up from the top of a chapter goes back to the end of the one
    // before it, the mirror of how we got here.
    let crossed = reader.apply(Command::Scroll(-50.0)).unwrap();
    assert_eq!(crossed.chapter, 0);
    assert_eq!(crossed.transition, Some(Transition::Backward));
    assert!(crossed.scroll > 0.0, "lands at the end of the chapter");
}

#[test]
fn dragging_the_scrollbar_lands_where_it_says() {
    let long = support::paragraphs(60);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);

    let middle = reader.apply(Command::ScrollTo(0.5)).unwrap();
    let max = middle.chapter_height - middle.viewport_height;
    assert!((middle.scroll - max * 0.5).abs() < 1.0, "half way down");

    let end = reader.apply(Command::ScrollTo(1.0)).unwrap();
    assert!((end.scroll - max).abs() < 1.0, "the very end");

    // Out-of-range fractions clamp rather than crossing chapters.
    let clamped = reader.apply(Command::ScrollTo(9.0)).unwrap();
    assert_eq!(clamped.chapter, 0);
    assert!((clamped.scroll - max).abs() < 1.0);
    let start = reader.apply(Command::ScrollTo(-3.0)).unwrap();
    assert_eq!(start.scroll, 0.0);
}

#[test]
fn the_start_of_the_book_does_not_move() {
    let mut reader = reader_with(vec![("one.xhtml", "<p>only</p>")]);
    let view = reader.apply(Command::Scroll(-500.0)).unwrap();
    assert_eq!((view.chapter, view.scroll), (0, 0.0));
    assert!(view.transition.is_none());
}

#[test]
fn a_band_is_reused_while_scrolling_inside_it() {
    let long = support::paragraphs(60);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);

    let first = reader.apply(Command::Refresh).unwrap();
    assert!(first.band.is_some(), "the first frame paints");

    // A nudge stays inside the painted band: no new pixels, just an offset.
    let nudge = reader.apply(Command::Scroll(8.0)).unwrap();
    assert!(
        nudge.band.is_none(),
        "scrolling inside the band must not repaint"
    );
    assert_eq!(nudge.scroll, 8.0);

    // Scrolling far enough demands a fresh band.
    let far = reader.apply(Command::Scroll(5_000.0)).unwrap();
    assert!(far.band.is_some(), "leaving the band paints a new one");
    assert!(far.band_top > 0.0);
}

#[test]
fn zooming_keeps_the_reader_in_the_same_place() {
    let long = support::paragraphs(60);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);

    reader.apply(Command::ScrollScreens(3.0)).unwrap();
    let before = reader.apply(Command::Refresh).unwrap();
    let anchor = before.position;

    let zoomed = reader.apply(Command::Zoom(2)).unwrap();
    assert!(zoomed.font_px > before.font_px, "font actually grew");
    assert_eq!(
        zoomed.position.block, anchor.block,
        "the block at the top of the window survives a zoom"
    );
    assert!(
        zoomed.chapter_height > before.chapter_height,
        "bigger text makes a taller column"
    );

    let back = reader.apply(Command::ZoomReset).unwrap();
    assert_eq!(back.font_px, before.font_px);
    assert_eq!(back.position.block, anchor.block);
}

#[test]
fn focus_mode_narrows_the_column_without_losing_the_place() {
    let long = support::paragraphs(40);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);
    reader.apply(Command::ScrollScreens(2.0)).unwrap();
    let before = reader.apply(Command::Refresh).unwrap();

    let focused = reader.apply(Command::SetFocus(true)).unwrap();
    assert!(focused.focus);
    assert_eq!(focused.position.block, before.position.block);
    assert!(
        focused.chapter_height >= before.chapter_height,
        "a narrower measure cannot make a shorter column"
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
    assert_eq!(
        start.links.as_ref().map(Vec::len),
        Some(1),
        "the link is hit-testable"
    );
    assert!(!start.can_go_back);

    let jumped = reader
        .apply(Command::FollowLink("two.xhtml#target".into()))
        .unwrap();
    assert_eq!(jumped.chapter, 1);
    assert!(jumped.scroll > 0.0, "the anchor is past the first screen");
    assert_eq!(jumped.transition, Some(Transition::Forward));
    assert!(jumped.can_go_back);

    // The offset shown is the one that holds the anchored block.
    let anchor_block = 40;
    let direct = reader
        .apply(Command::GoTo(Position {
            chapter: 1,
            block: anchor_block,
            char_offset: 0,
        }))
        .unwrap();
    assert!(
        (direct.scroll - jumped.scroll).abs() < 1.0,
        "link and position agree: {} vs {}",
        direct.scroll,
        jumped.scroll
    );

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
    assert!(view.scroll > 0.0, "the heading is past the first screen");
    assert_eq!(
        view.toc_index,
        Some(1),
        "the contents panel highlights the entry we are inside"
    );

    let back_to_front = reader.apply(Command::GoToToc(0)).unwrap();
    assert_eq!(back_to_front.scroll, 0.0);
    assert_eq!(back_to_front.toc_index, Some(0));
}

#[test]
fn progress_is_weighted_by_how_long_each_chapter_is() {
    // A title page and a long chapter: counting spine entries would call the
    // start of the second chapter "half way through the book".
    let long = support::paragraphs(60);
    let mut reader = reader_with(vec![
        ("title.xhtml", "<h1>A Title Page</h1>"),
        ("one.xhtml", long.as_str()),
    ]);

    let start = reader.apply(Command::Refresh).unwrap();
    assert!(start.progress < 0.02, "the very start of the book");

    let second = reader.apply(Command::Chapter(1)).unwrap();
    assert_eq!(second.chapter, 1);
    assert!(
        second.progress < 0.2,
        "a title page is a sliver of the book, not half of it: got {}",
        second.progress
    );

    let end = reader.apply(Command::ScrollTo(1.0)).unwrap();
    assert!(end.progress > 0.9, "the end of the last chapter is the end");
    assert!(end.progress <= 1.0);
}

#[test]
fn the_time_left_shrinks_as_the_chapter_is_read() {
    let long = support::paragraphs(80);
    let mut reader = reader_with(vec![("one.xhtml", long.as_str())]);

    let start = reader.apply(Command::Refresh).unwrap();
    let at_start = start.minutes_left.expect("a long chapter takes minutes");

    let later = reader.apply(Command::ScrollTo(0.8)).unwrap();
    // Under a minute left is reported as nothing at all, which is also fine.
    if let Some(minutes) = later.minutes_left {
        assert!(
            minutes < at_start,
            "four fifths in should be quicker than the start: {minutes} vs {at_start}"
        );
    }

    let end = reader.apply(Command::ScrollTo(1.0)).unwrap();
    assert_eq!(end.minutes_left, None, "nothing left to say at the end");
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

    let light_band = light.band.expect("light frame paints");
    let dark_band = dark.band.expect("a theme change repaints");
    assert_eq!(light_band.rgba.len(), dark_band.rgba.len());
    assert_ne!(light_band.rgba, dark_band.rgba, "the band is repainted");
    // Taken from the theme rather than hard-coded, so tuning the palette does
    // not break the test that only cares that the page is painted with it.
    let dark_page = Theme::new(ThemePreset::Dark).page;
    assert_eq!(
        &dark_band.rgba[..4],
        &[dark_page.r, dark_page.g, dark_page.b, 0xFF]
    );
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
