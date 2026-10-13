//! Render what the window would show, to a PNG, with no window involved.
//!
//! Usage:
//!   cargo run -p folio-core --example dump_page -- BOOK.epub \
//!       [--chapter N] [--screens N] [--theme light|sepia|dark] [--zoom N]
//!       [--focus] [--spotlight] [--justify] [--size WxH] [--out FILE]

use std::path::PathBuf;

use folio_core::layout::Alignment;
use folio_core::reader::{Command, Reader};
use folio_core::state::{Position, Settings};
use folio_core::theme::{Theme, ThemePreset};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let book = PathBuf::from(args.next().ok_or("usage: dump_page BOOK.epub [options]")?);

    let mut chapter = 1usize;
    let mut screens = 0.0_f32;
    let mut preset = ThemePreset::Light;
    let mut zoom = 0i32;
    let mut focus = false;
    let mut spotlight = false;
    let mut justify = false;
    let mut size = (1000u32, 760u32);
    let mut out = String::from("page.png");

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--chapter" => chapter = args.next().unwrap_or_default().parse()?,
            "--screens" => screens = args.next().unwrap_or_default().parse()?,
            "--zoom" => zoom = args.next().unwrap_or_default().parse()?,
            "--theme" => {
                preset = match args.next().unwrap_or_default().as_str() {
                    "dark" => ThemePreset::Dark,
                    "sepia" => ThemePreset::Sepia,
                    _ => ThemePreset::Light,
                }
            }
            "--focus" => focus = true,
            "--spotlight" => spotlight = true,
            "--justify" => justify = true,
            "--size" => {
                let value = args.next().unwrap_or_default();
                let (w, h) = value
                    .split_once('x')
                    .ok_or("size must look like 1000x760")?;
                size = (w.parse()?, h.parse()?);
            }
            "--out" => out = args.next().unwrap_or_default(),
            other => return Err(format!("unknown flag {other}").into()),
        }
    }

    let mut settings = Settings {
        spotlight,
        ..Settings::default()
    };
    if justify {
        settings.type_settings.alignment = Alignment::Justified;
    }

    let mut reader = Reader::open(&book, &settings, preset, size, None)?;
    println!(
        "{} · {} chapters · {} toc entries",
        reader.meta().display_line(),
        reader.chapter_count(),
        reader.toc().len()
    );

    reader.apply(Command::SetTheme(Theme::new(preset)))?;
    reader.apply(Command::GoTo(Position {
        chapter,
        block: 0,
        char_offset: 0,
    }))?;
    if zoom != 0 {
        reader.apply(Command::Zoom(zoom))?;
    }
    if focus {
        reader.apply(Command::SetFocus(true))?;
    }
    if spotlight {
        reader.apply(Command::SetSpotlight(true))?;
        reader.apply(Command::MoveSpotlight(2))?;
    }
    let view = if screens != 0.0 {
        reader.apply(Command::ScrollScreens(screens))?
    } else {
        reader.apply(Command::Refresh)?
    };

    println!(
        "chapter {} '{}' · scroll {:.0}/{:.0}px · {} links · {:.0}% of book · {}px type",
        view.chapter,
        view.chapter_label,
        view.scroll,
        view.chapter_height,
        view.links.as_ref().map(Vec::len).unwrap_or(0),
        view.progress * 100.0,
        view.font_px
    );

    // The band is taller than the window; crop the part that would be visible.
    // A frame carries no band when the one already on screen still covers the
    // viewport, which is the normal case for a small scroll — so ask for a
    // repaint rather than asserting.
    let view = match view.band {
        Some(_) => view,
        // A theme change always repaints, and this one changes nothing.
        None => reader.apply(Command::SetTheme(Theme::new(preset)))?,
    };
    let band = view.band.expect("a theme change repaints");
    let offset = ((view.scroll - view.band_top).max(0.0)).round() as u32;
    let height = (view.viewport_height.round() as u32).min(band.height.saturating_sub(offset));
    let start = (offset * band.width * 4) as usize;
    let end = start + (height * band.width * 4) as usize;
    image::save_buffer(
        &out,
        &band.rgba[start..end.min(band.rgba.len())],
        band.width,
        height,
        image::ColorType::Rgba8,
    )?;
    println!("wrote {out}");
    Ok(())
}
