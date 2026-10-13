//! Folio — an EPUB reader.
//!
//! The window is a projection of `folio_core::reader::View`: every key, click
//! and button turns into a [`Command`] for the render worker, and every frame
//! that comes back replaces what is on screen.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod logging;
mod worker;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use folio_core::book::{TocItem, path as href_path};
use folio_core::reader::{Command, SearchHit};
use folio_core::state::{RecentBook, Settings};
use folio_core::store::Store;
use folio_core::theme::{Theme, ThemeChoice, ThemePreset};
use slint::ComponentHandle;

use worker::{Event, Frame, Job, Worker, next_preset};

slint::include_modules!();

/// Everything the UI thread keeps between callbacks.
struct App {
    ui: slint::Weak<AppWindow>,
    worker: Worker,
    store: Option<Store>,
    settings: RefCell<Settings>,
    theme: Cell<ThemeChoice>,
    preset: Cell<ThemePreset>,
    /// Generation of the most recent Open; older events belong to a book that
    /// is no longer on screen and are discarded.
    book_generation: Cell<u64>,
    /// Physical pixel size of the reading area.
    viewport: Cell<(u32, u32)>,
    scale: Cell<f32>,
    generation: Cell<u64>,
    frame: RefCell<Option<Box<Frame>>>,
    toc: RefCell<Vec<TocItem>>,
    hits: RefCell<Vec<SearchHit>>,
    recents: RefCell<Vec<RecentBook>>,
    has_book: Cell<bool>,
}

impl App {
    fn next_generation(&self) -> u64 {
        let next = self.generation.get() + 1;
        self.generation.set(next);
        next
    }

    /// Send a reading command to the worker.
    fn command(&self, command: Command) {
        if !self.has_book.get() {
            return;
        }
        let generation = self.next_generation();
        self.worker.send(Job::Apply {
            generation,
            command,
        });
    }

    fn open(&self, path: PathBuf) {
        let generation = self.next_generation();
        self.book_generation.set(generation);
        let settings = self.settings.borrow().clone();
        self.worker.send(Job::Open {
            generation,
            path,
            settings,
            theme: Theme::new(self.preset.get()),
            viewport: self.viewport.get(),
        });
    }

    fn apply_theme(&self, preset: ThemePreset) {
        self.preset.set(preset);
        let theme = Theme::new(preset);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_tokens(bridge::tokens(&theme));
            ui.set_theme_label(preset.label().into());
        }
        self.command(Command::SetTheme(theme));
        self.save_settings();
    }

    fn save_settings(&self) {
        let mut settings = self.settings.borrow_mut();
        settings.theme = self.theme.get();
        if let Some(ui) = self.ui.upgrade() {
            settings.toc_open = ui.get_toc_open();
            settings.window_width = ui.window().size().width.max(400);
            settings.window_height = ui.window().size().height.max(300);
        }
        if let Some(store) = &self.store
            && let Err(error) = store.save_settings(&settings)
        {
            tracing::warn!(%error, "could not save settings");
        }
    }

    fn notice(&self, message: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_notice(message.into());
        let weak = self.ui.clone();
        // Clear the message after a few seconds.
        let timer = Rc::new(slint::Timer::default());
        let keep = Rc::clone(&timer);
        timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(4),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_notice("".into());
                }
                let _ = &keep;
            },
        );
        std::mem::forget(timer);
    }

    /// Handle a click on the page: follow a link, or turn the page.
    fn page_clicked(&self, x: f32, y: f32) {
        let scale = self.scale.get();
        let (px, py) = (x * scale, y * scale);
        let link = self.frame.borrow().as_ref().and_then(|frame| {
            frame
                .links
                .iter()
                .find(|region| region.contains(px, py))
                .map(|region| region.href.clone())
        });
        match link {
            Some(href) if href_path::is_external(&href) => {
                if let Err(error) = open::that_detached(&href) {
                    tracing::warn!(%error, %href, "could not open link");
                    self.notice("Could not open that link");
                }
            }
            Some(href) => self.command(Command::FollowLink(href)),
            None => {
                let width = self.viewport.get().0 as f32;
                self.command(Command::Page(if px < width * 0.33 { -1 } else { 1 }));
            }
        }
    }

    /// Paint a frame that came back from the worker.
    fn show_frame(&self, frame: Box<Frame>) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_page_image(bridge::image_from_rgba(
            &frame.rgba,
            frame.width,
            frame.height,
        ));
        ui.set_page(frame.page as i32);
        ui.set_page_count(frame.page_count.max(1) as i32);
        ui.set_progress(frame.progress);
        ui.set_chapter_label(frame.chapter_label.as_str().into());
        ui.set_focus_mode(frame.focus);
        ui.set_spotlight(frame.spotlight);
        ui.set_can_go_back(frame.can_go_back);
        ui.set_font_px(frame.font_px.round() as i32);
        ui.set_toc(bridge::toc_rows(&self.toc.borrow(), frame.toc_index));
        self.settings.borrow_mut().type_settings.font_px = frame.font_px;
        *self.frame.borrow_mut() = Some(frame);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_log_guard, _log_dir) = logging::init();

    let store = Store::new().ok();
    let settings = store
        .as_ref()
        .map(|store| store.load_settings())
        .unwrap_or_default();
    let recents = store
        .as_ref()
        .map(|store| store.load_recents())
        .unwrap_or_default();

    // Wayland and X11 use this to match the window to folio.desktop.
    if let Err(error) = slint::set_xdg_app_id("folio") {
        tracing::warn!(%error, "could not set the XDG app id");
    }

    let ui = AppWindow::new()?;
    let preset = settings.theme.resolve(ui.get_system_dark());

    let app = Rc::new_cyclic(|_weak: &std::rc::Weak<App>| App {
        ui: ui.as_weak(),
        worker: spawn_worker(ui.as_weak()),
        store,
        settings: RefCell::new(settings.clone()),
        theme: Cell::new(settings.theme),
        preset: Cell::new(preset),
        book_generation: Cell::new(0),
        viewport: Cell::new((1000, 700)),
        scale: Cell::new(ui.window().scale_factor()),
        generation: Cell::new(0),
        frame: RefCell::new(None),
        toc: RefCell::new(Vec::new()),
        hits: RefCell::new(Vec::new()),
        recents: RefCell::new(recents.clone()),
        has_book: Cell::new(false),
    });

    ui.set_tokens(bridge::tokens(&Theme::new(preset)));
    ui.set_theme_label(preset.label().into());
    ui.set_font_px(settings.type_settings.font_px.round() as i32);
    ui.set_toc_open(settings.toc_open);
    if let Some(store) = &app.store {
        ui.set_shelf(bridge::shelf_rows(&recents, &store.covers_dir()));
    }
    ui.window().set_size(slint::PhysicalSize::new(
        settings.window_width,
        settings.window_height,
    ));

    wire_callbacks(&ui, &app);

    // A path on the command line opens straight away.
    if let Some(path) = std::env::args_os().nth(1) {
        app.open(PathBuf::from(path));
    }

    ui.run()?;
    app.save_settings();
    app.worker.send(Job::Shutdown);
    Ok(())
}

/// Start the worker and marshal its events back onto the UI thread.
fn spawn_worker(ui: slint::Weak<AppWindow>) -> Worker {
    worker::spawn(move |event| {
        let ui = ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui.upgrade() else { return };
            // The App handle is stored in the window's user data via callbacks;
            // events are applied through the shared handler below.
            HANDLER.with(|handler| {
                if let Some(handler) = handler.borrow().as_ref() {
                    handler(event);
                }
            });
            let _ = ui;
        });
    })
}

/// Applies a worker event to the UI; installed once the [`App`] exists.
type EventHandler = Box<dyn Fn(Event)>;

thread_local! {
    /// Set once the App exists, so worker events can reach it.
    static HANDLER: RefCell<Option<EventHandler>> = const { RefCell::new(None) };
}

fn wire_callbacks(ui: &AppWindow, app: &Rc<App>) {
    // Worker events.
    {
        let app = Rc::clone(app);
        HANDLER.with(|handler| {
            *handler.borrow_mut() = Some(Box::new(move |event| handle_event(&app, event)));
        });
    }

    macro_rules! on {
        ($setter:ident, $body:expr) => {{
            let app = Rc::clone(app);
            ui.$setter($body(app));
        }};
    }

    on!(on_turn_page, |app: Rc<App>| move |delta: i32| {
        app.command(Command::Page(delta));
    });
    on!(on_jump_chapter, |app: Rc<App>| move |delta: i32| {
        app.command(Command::Chapter(delta));
    });
    on!(on_toggle_focus, |app: Rc<App>| move || {
        let focus = app
            .frame
            .borrow()
            .as_ref()
            .map(|frame| frame.focus)
            .unwrap_or(false);
        app.command(Command::SetFocus(!focus));
    });
    on!(on_toggle_spotlight, |app: Rc<App>| move || {
        let spotlight = !app.settings.borrow().spotlight;
        app.settings.borrow_mut().spotlight = spotlight;
        app.command(Command::SetSpotlight(spotlight));
        app.save_settings();
    });
    on!(on_move_spotlight, |app: Rc<App>| move |delta: i32| {
        app.command(Command::MoveSpotlight(delta));
    });
    on!(on_cycle_theme, |app: Rc<App>| move || {
        let next = next_preset(app.preset.get());
        app.theme.set(ThemeChoice::Fixed(next));
        app.apply_theme(next);
    });
    on!(on_zoom, |app: Rc<App>| move |step: i32| {
        app.command(Command::Zoom(step));
    });
    on!(on_zoom_reset, |app: Rc<App>| move || {
        app.command(Command::ZoomReset);
    });
    on!(on_back, |app: Rc<App>| move || {
        app.command(Command::Back);
    });
    on!(on_goto_toc, |app: Rc<App>| move |index: i32| {
        app.command(Command::GoToToc(index.max(0) as usize));
    });
    on!(on_goto_search, |app: Rc<App>| move |index: i32| {
        let position = app
            .hits
            .borrow()
            .get(index.max(0) as usize)
            .map(|hit| hit.position);
        if let Some(position) = position {
            app.command(Command::GoTo(position));
        }
    });
    on!(on_run_search, |app: Rc<App>| {
        move |query: slint::SharedString| {
            if !app.has_book.get() {
                return;
            }
            let generation = app.next_generation();
            if let Some(ui) = app.ui.upgrade() {
                ui.set_searching(true);
            }
            app.worker.send(Job::Search {
                generation,
                query: query.to_string(),
            });
        }
    });
    on!(on_page_clicked, |app: Rc<App>| move |x: f32, y: f32| {
        app.page_clicked(x, y);
    });
    on!(on_viewport_changed, |app: Rc<App>| move |w: f32, h: f32| {
        let scale = app
            .ui
            .upgrade()
            .map(|ui| ui.window().scale_factor())
            .unwrap_or(1.0);
        app.scale.set(scale);
        let size = (
            (w * scale).round().max(1.0) as u32,
            (h * scale).round().max(1.0) as u32,
        );
        if app.viewport.get() == size {
            return;
        }
        app.viewport.set(size);
        app.command(Command::Resize {
            width: size.0,
            height: size.1,
        });
    });
    on!(on_open_file, |app: Rc<App>| move || {
        let picked = rfd::FileDialog::new()
            .add_filter("EPUB books", &["epub"])
            .set_title("Open a book")
            .pick_file();
        if let Some(path) = picked {
            app.open(path);
        }
    });
    on!(on_open_recent, |app: Rc<App>| move |index: i32| {
        let path = app
            .recents
            .borrow()
            .get(index.max(0) as usize)
            .map(|book| PathBuf::from(&book.path));
        if let Some(path) = path {
            app.open(path);
        }
    });
    on!(on_quit, |app: Rc<App>| move || {
        app.save_settings();
        app.worker.send(Job::Flush);
        let _ = slint::quit_event_loop();
    });
}

fn handle_event(app: &Rc<App>, event: Event) {
    // Frames produced for a previously open book must not overwrite the
    // current one; the worker and the UI agree on monotonic generations.
    let generation = match &event {
        Event::Opened { generation, .. }
        | Event::Frame { generation, .. }
        | Event::SearchDone { generation, .. }
        | Event::Failed { generation, .. } => *generation,
    };
    if generation < app.book_generation.get() {
        tracing::debug!(generation, "dropping stale worker event");
        return;
    }

    match event {
        Event::Opened {
            title,
            toc,
            recents,
            ..
        } => {
            *app.toc.borrow_mut() = toc;
            *app.recents.borrow_mut() = recents.clone();
            app.has_book.set(true);
            if let Some(ui) = app.ui.upgrade() {
                ui.set_book_title(title.as_str().into());
                ui.set_has_book(true);
                if let Some(store) = &app.store {
                    ui.set_shelf(bridge::shelf_rows(&recents, &store.covers_dir()));
                }
                ui.invoke_focus_reader();
            }
        }
        Event::Frame { frame, .. } => app.show_frame(frame),
        Event::SearchDone { hits, .. } => {
            if let Some(ui) = app.ui.upgrade() {
                ui.set_search_results(bridge::search_rows(&hits));
                ui.set_searching(false);
            }
            *app.hits.borrow_mut() = hits;
        }
        Event::Failed { message, .. } => {
            tracing::error!(%message, "command failed");
            app.notice(&message);
        }
    }
}
