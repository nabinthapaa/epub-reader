//! The band's whole purpose is that scrolling slides pixels instead of asking
//! for new ones. That only holds if the band is meaningfully taller than the
//! viewport — and if the memory cap never quietly shrinks it below that.

mod support;

use folio_core::reader::{Command, Reader};
use folio_core::state::Settings;
use folio_core::theme::ThemePreset;
use support::FakeBook;

fn reader_at(viewport: (u32, u32)) -> Reader {
    let long = support::paragraphs(200);
    Reader::with_book(
        Box::new(FakeBook::new(vec![("one.xhtml", long.as_str())])),
        "/tmp/band.epub".into(),
        &Settings::default(),
        ThemePreset::Light,
        viewport,
        None,
    )
    .expect("reader opens")
}

#[test]
fn the_band_always_covers_the_viewport_and_both_margins() {
    // Every one of these is a real display. The two largest used to fall below
    // the margin, which turned every wheel tick into a full repaint, and the
    // largest also blew through the byte budget.
    for (width, height) in [
        (1280u32, 800u32),
        (1600, 900),
        (1920, 1080),
        (2560, 1440),
        (3440, 1440),
        (3840, 2160),
        (5120, 2880),
    ] {
        let mut reader = reader_at((width, height));
        let view = reader.apply(Command::Refresh).unwrap();
        let band = view.band.expect("the first frame paints");

        let viewport = height as f32;
        assert!(
            band.height as f32 >= viewport * 1.8 - 1.0,
            "{width}x{height}: band {} is under the viewport plus both margins",
            band.height
        );
        assert_eq!(band.width, width);
    }
}

#[test]
fn scrolling_inside_the_band_does_not_repaint_but_leaving_it_does() {
    let mut reader = reader_at((1200, 800));
    // Away from the chapter's start, where the band is clamped to the top.
    let arrival = reader.apply(Command::ScrollTo(0.5)).unwrap();
    assert!(arrival.band.is_some(), "arriving somewhere new repaints");
    assert!(
        reader.apply(Command::Refresh).unwrap().band.is_none(),
        "and asking again reuses what was just painted"
    );

    let nudge = reader.apply(Command::Scroll(40.0)).unwrap();
    assert!(nudge.band.is_none(), "a nudge slides the painted band");

    let far = reader.apply(Command::Scroll(800.0)).unwrap();
    assert!(far.band.is_some(), "leaving the band paints a new one");
}

#[test]
fn a_screen_jump_stays_inside_the_painted_band() {
    // The UI eases rather than snaps for anything under EASE_LIMIT_FRACTION of
    // a viewport, so anything it eases has to be covered by the band's pre-roll
    // or the window shows bare page for a few frames.
    let viewport = 800.0_f32;
    let eased = viewport * folio_core::reader::EASE_LIMIT_FRACTION;
    let pre_roll = viewport * (2.2 - 1.0) / 2.0;
    assert!(
        eased <= pre_roll,
        "an eased jump of {eased}px would outrun {pre_roll}px of pre-roll"
    );

    let mut reader = reader_at((1200, viewport as u32));
    let view = reader.apply(Command::ScrollScreens(1.0)).unwrap();
    assert!(
        view.scroll - view.band_top >= 0.0,
        "the viewport starts inside the band"
    );
}
