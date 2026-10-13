//! Render pages to PNG without a window, for inspecting layout and themes.
//!
//! Usage:
//!   cargo run -p folio-core --example dump_page -- BOOK.epub \
//!       [--chapter N] [--page N] [--theme light|sepia|dark] [--zoom N]
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
    let mut page = 0usize;
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
            "--page" => page = args.next().unwrap_or_default().parse()?,
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
    let view = if page > 0 {
        reader.apply(Command::Page(page as i32))?
    } else {
        reader.apply(Command::Refresh)?
    };

    println!(
        "chapter {} '{}' · page {}/{} · {} links · {:.0}% · {}px",
        view.chapter,
        view.chapter_label,
        view.page + 1,
        view.page_count,
        view.links.len(),
        view.progress * 100.0,
        view.font_px
    );
    image::save_buffer(
        &out,
        &view.canvas.rgba,
        view.canvas.width,
        view.canvas.height,
        image::ColorType::Rgba8,
    )?;
    println!("wrote {out}");
    Ok(())
}
