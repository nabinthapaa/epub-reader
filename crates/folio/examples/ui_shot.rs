//! Render the window to a PNG without a compositor.
//!
//! Slint's testing backend can embed a real software rasterizer, so the whole
//! UI — chrome, panels and the painted band — can be captured deterministically
//! from a test machine or CI, instead of photographing someone's desktop.
//!
//! Usage:
//!   cargo run -p folio --example ui_shot -- shelf   out.png [WxH]
//!   cargo run -p folio --example ui_shot -- reading out.png [WxH] [BOOK.epub]
//!   cargo run -p folio --example ui_shot -- settings|toc|search|focus …

use std::path::{Path, PathBuf};

use folio_core::layout::raster::LinkRegion;
use folio_core::reader::{Command, Reader};
use folio_core::state::Settings;
use folio_core::store::Store;
use folio_core::theme::{Theme, ThemePreset};
use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let scene = args.next().unwrap_or_else(|| "shelf".into());
    let out = args.next().unwrap_or_else(|| "shot.png".into());
    let size = args.next().unwrap_or_else(|| "1100x820".into());
    let book = args.next().map(PathBuf::from);
    let (width, height) = size
        .split_once('x')
        .map(|(w, h)| (w.parse::<u32>().unwrap(), h.parse::<u32>().unwrap()))
        .expect("size looks like 1100x820");

    slint::platform::set_platform(Box::new(TestingBackend::new(TestingBackendOptions {
        mock_time: true,
        threading: false,
        renderer_name: Some("software".into()),
    })))
    .map_err(|e| format!("could not install the headless backend: {e}"))?;

    let ui = AppWindow::new()?;
    ui.window()
        .set_size(slint::PhysicalSize::new(width, height));

    let preset = match std::env::var("FOLIO_SHOT_THEME").as_deref() {
        Ok("light") => ThemePreset::Light,
        Ok("sepia") => ThemePreset::Sepia,
        _ => ThemePreset::Dark,
    };
    let theme = Theme::new(preset);
    ui.set_tokens(tokens(&theme));
    ui.set_theme_label(preset.label().into());

    // The shelf reads the real recents, so shots show real covers.
    if let Ok(store) = Store::new() {
        let recents = store.load_recents();
        let rows: Vec<ShelfRow> = recents
            .iter()
            .enumerate()
            .map(|(index, book)| ShelfRow {
                title: book.title.as_str().into(),
                author: book.author.as_str().into(),
                progress: book.progress_percent() as i32,
                cover: book
                    .cover
                    .as_ref()
                    .and_then(|name| {
                        slint::Image::load_from_path(&store.covers_dir().join(name)).ok()
                    })
                    .unwrap_or_default(),
                index: index as i32,
            })
            .collect();
        ui.set_shelf(ModelRc::new(VecModel::from(rows)));
    }

    if scene != "shelf" {
        let path = book
            .or_else(|| {
                Store::new()
                    .ok()
                    .and_then(|store| store.load_recents().first().map(|b| PathBuf::from(&b.path)))
            })
            .ok_or("no book to render: pass one as the last argument")?;
        load_book(&ui, &path, theme, (width, height))?;
    }

    match scene.as_str() {
        "toc" => ui.set_toc_open(true),
        "search" => {
            ui.set_search_open(true);
            ui.set_search_results(ModelRc::new(VecModel::from(vec![
                SearchRow {
                    label: "CHAPTER 32. Cetology.".into(),
                    snippet: "…the Sperm Whale, which is the largest of the whales…".into(),
                    index: 0,
                },
                SearchRow {
                    label: "CHAPTER 41. Moby Dick.".into(),
                    snippet: "…a whale of uncommon magnitude and malignity…".into(),
                    index: 1,
                },
            ])));
        }
        "settings" => {
            ui.set_settings_open(true);
            ui.set_families(ModelRc::new(VecModel::from(vec![
                SharedString::from("Charis SIL"),
                SharedString::from("Noto Serif"),
                SharedString::from("Noto Sans"),
            ])));
            ui.set_family_index(1);
        }
        "focus" => ui.set_focus_mode(true),
        _ => {}
    }

    // Let bindings settle, then rasterize.
    slint::platform::update_timers_and_animations();
    let buffer = ui.window().take_snapshot()?;
    image::save_buffer(
        &out,
        buffer.as_bytes(),
        buffer.width(),
        buffer.height(),
        image::ColorType::Rgba8,
    )?;
    println!("wrote {out} ({}x{})", buffer.width(), buffer.height());
    Ok(())
}

/// Open a book and push one painted band into the window.
fn load_book(
    ui: &AppWindow,
    path: &Path,
    theme: Theme,
    viewport: (u32, u32),
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::default();
    let mut reader = Reader::open(path, &settings, theme.preset, viewport, None)?;
    reader.apply(Command::SetTheme(theme))?;
    // A few screens in, so the shot shows prose rather than a title page.
    let view = reader.apply(Command::ScrollScreens(4.0))?;

    ui.set_has_book(true);
    ui.set_book_title(reader.meta().display_line().as_str().into());
    ui.set_chapter_label(view.chapter_label.as_str().into());
    ui.set_progress(view.progress);
    ui.set_chapter_progress(view.chapter_progress);
    ui.set_font_px(view.font_px.round() as i32);
    ui.set_scroll(view.scroll);
    ui.set_band_top(view.band_top);
    ui.set_chapter_height(view.chapter_height);
    ui.set_viewport_fraction(
        (view.viewport_height / view.chapter_height.max(1.0)).clamp(0.05, 1.0),
    );

    if let Some(band) = view.band {
        let mut buffer =
            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(band.width, band.height);
        buffer.make_mut_bytes().copy_from_slice(&band.rgba);
        ui.set_band_image(slint::Image::from_rgba8(buffer));
        ui.set_band_height(band.height as f32);
    }

    let rows: Vec<TocRow> = reader
        .toc()
        .iter()
        .enumerate()
        .map(|(index, item)| TocRow {
            label: item.label.as_str().into(),
            depth: item.depth.min(4) as i32,
            index: index as i32,
            current: Some(index) == view.toc_index,
        })
        .collect();
    ui.set_toc(ModelRc::new(VecModel::from(rows)));
    let _: Option<Vec<LinkRegion>> = view.links;
    Ok(())
}

fn tokens(theme: &Theme) -> Tokens {
    let color = |c: folio_core::theme::Color| slint::Color::from_argb_u8(c.a, c.r, c.g, c.b);
    Tokens {
        page: color(theme.page),
        surface: color(theme.surface),
        text: color(theme.text),
        muted: color(theme.muted),
        accent: color(theme.accent),
        link: color(theme.link),
        border: color(theme.border),
    }
}
