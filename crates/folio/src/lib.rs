//! Folio — an EPUB reader.
//!
//! The window is a projection of `folio_core::reader::View`: every key, click
//! and button turns into a [`Command`] for the render worker, and every frame
//! that comes back replaces what is on screen.
//!
//! This is a library so that the desktop binary and the Android entry point
//! can share all of it; only the way the app is started differs.

pub mod bridge;
pub mod logging;
pub mod worker;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use folio_core::book::{TocItem, path as href_path};
use folio_core::layout::raster::LinkRegion;
use folio_core::reader::{Command, SearchHit, Transition};
use folio_core::state::{Position, RecentBook, Settings};
use folio_core::store::Store;
use folio_core::theme::{Theme, ThemeChoice, ThemePreset};
use slint::ComponentHandle;

use worker::{Event, Frame, Job, Worker, next_preset};

slint::include_modules!();

/// Everything the UI thread keeps between callbacks.
struct App {
    /// A handle to itself, for the timers that outlive a callback.
    me: std::rc::Weak<App>,
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
    /// Hit-testable links of the band on screen, in chapter space.
    links: RefCell<Vec<LinkRegion>>,
    /// Scroll offsets in physical pixels: where we are heading, and where the
    /// view currently is while it eases there.
    scroll_target: Cell<f32>,
    scroll_visual: Cell<f32>,
    scroll_timer: RefCell<Option<slint::Timer>>,
    transition_timer: RefCell<Option<slint::Timer>>,
    chapter: Cell<usize>,
    toc: RefCell<Vec<TocItem>>,
    families: RefCell<Vec<String>>,
    /// Kept alive so the transient message clears itself exactly once.
    notice_timer: RefCell<Option<slint::Timer>>,
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
        if !self.worker.send(Job::Apply {
            generation,
            command,
        }) {
            // The worker died; without saying so the window just stops
            // responding to every key with no explanation.
            self.notice("The renderer stopped — see the log");
        }
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

    /// Put the book away and show the shelf again.
    fn close_book(&self) {
        if !self.has_book.get() {
            return;
        }
        self.has_book.set(false);
        // Frames of the book being closed must not land on the shelf.
        self.book_generation.set(self.next_generation());
        self.worker.send(Job::CloseBook);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_has_book(false);
            // Dropping the transition timer stops the animation without
            // settling it; the next book would open dimmed and offset.
            ui.set_enter_progress(1.0);
            ui.set_book_title("Folio".into());
            ui.set_book_author("".into());
            ui.set_search_open(false);
            ui.set_settings_open(false);
            ui.set_can_go_back(false);
            ui.invoke_focus_reader();
        }
        self.links.borrow_mut().clear();
        self.toc.borrow_mut().clear();
        self.hits.borrow_mut().clear();
        *self.frame.borrow_mut() = None;
        self.scroll_timer.borrow_mut().take();
        self.transition_timer.borrow_mut().take();
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
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(4),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_notice("".into());
                }
            },
        );
        *self.notice_timer.borrow_mut() = Some(timer);
    }

    /// Change a typographic setting, re-render with it, and remember it.
    fn edit_type_settings(&self, edit: impl FnOnce(&mut folio_core::TypeSettings)) {
        let settings = {
            let mut settings = self.settings.borrow_mut();
            edit(&mut settings.type_settings);
            settings.type_settings.clone()
        };
        if let Some(ui) = self.ui.upgrade() {
            ui.set_justify(matches!(
                settings.alignment,
                folio_core::layout::Alignment::Justified
            ));
            ui.set_measure(settings.measure_ch.round() as i32);
            ui.set_family_index(self.family_index(settings.family.as_deref()));
        }
        self.command(Command::SetTypeSettings(settings));
        self.save_settings();
    }

    /// Row of the typeface list matching `family`.
    fn family_index(&self, family: Option<&str>) -> i32 {
        let Some(family) = family else { return -1 };
        self.families
            .borrow()
            .iter()
            .position(|installed| installed == family)
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    /// Handle a click on the text: follow a link if one is under the pointer.
    fn page_clicked(&self, x: f32, y: f32) {
        let scale = self.scale.get();
        // Viewport space -> chapter space, which is where links live.
        let px = x * scale;
        let py = y * scale + self.scroll_visual.get();
        let link = self
            .links
            .borrow()
            .iter()
            .find(|region| region.contains(px, py))
            .map(|region| region.href.clone());
        match link {
            Some(href) if href_path::is_external(&href) => self.open_externally(&href),
            Some(href) => self.command(Command::FollowLink(href)),
            None => {}
        }
    }

    /// Hand an external link to the system browser.
    #[cfg(not(target_os = "android"))]
    fn open_externally(&self, href: &str) {
        if let Err(error) = open::that_detached(href) {
            tracing::warn!(%error, %href, "could not open link");
            self.notice("Could not open that link");
        }
    }

    /// Opening a link on Android needs an Intent through JNI. Until that
    /// exists, say so rather than appearing to do nothing.
    #[cfg(target_os = "android")]
    fn open_externally(&self, href: &str) {
        tracing::info!(%href, "external links are not wired up on Android yet");
        self.notice("Opening links is not supported here yet");
    }

    /// Ask the platform for a book to open.
    #[cfg(not(target_os = "android"))]
    fn pick_book(&self) {
        let picked = rfd::FileDialog::new()
            .add_filter("EPUB books", &["epub"])
            .set_title("Open a book")
            .pick_file();
        if let Some(path) = picked {
            self.open(path);
        }
    }

    /// Android has no file dialog here yet, so books are picked up from the
    /// app's own storage: it needs no permission, and a file manager or a USB
    /// cable can put books there.
    #[cfg(target_os = "android")]
    fn pick_book(&self) {
        match self.books_in_storage().first() {
            Some(path) => self.open(path.clone()),
            None => self.notice("Copy .epub files into the Folio folder to read them"),
        }
    }

    /// Every EPUB sitting in the app's storage.
    #[cfg(target_os = "android")]
    fn books_in_storage(&self) -> Vec<PathBuf> {
        let Some(root) = self.books_dir.clone() else {
            return Vec::new();
        };
        let mut books: Vec<PathBuf> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"))
            })
            .collect();
        books.sort();
        books
    }

    /// Ease the view towards `target`, or jump straight there.
    ///
    /// Easing is for reading: wheel ticks, keys, a nudge. A jump of more than a
    /// screen is a navigation, and gliding through hundreds of lines would both
    /// look wrong and outrun the painted band, leaving the window blank.
    fn set_scroll(&self, target: f32, snap: bool) {
        let viewport = self
            .frame
            .borrow()
            .as_ref()
            .map(|frame| frame.viewport_height)
            .unwrap_or(self.viewport.get().1 as f32);
        // Easing further than the band's pre-roll would slide the painted
        // pixels off the top of the window and show bare page for a few frames.
        let snap = snap
            || (target - self.scroll_visual.get()).abs()
                > viewport * folio_core::reader::EASE_LIMIT_FRACTION;
        self.scroll_target.set(target);
        if snap {
            self.scroll_visual.set(target);
            self.scroll_timer.borrow_mut().take();
            self.push_scroll();
            return;
        }
        if (self.scroll_visual.get() - target).abs() < 0.5 {
            return;
        }
        if self.scroll_timer.borrow().is_some() {
            return;
        }
        let weak = self.me.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                let Some(app) = weak.upgrade() else { return };
                let target = app.scroll_target.get();
                let current = app.scroll_visual.get();
                let delta = target - current;
                if delta.abs() < 0.5 {
                    app.scroll_visual.set(target);
                    app.push_scroll();
                    app.scroll_timer.borrow_mut().take();
                    return;
                }
                // Exponential ease: fast at first, settling softly.
                app.scroll_visual.set(current + delta * 0.28);
                app.push_scroll();
            },
        );
        *self.scroll_timer.borrow_mut() = Some(timer);
    }

    fn push_scroll(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let scale = self.scale.get().max(0.1);
            ui.set_scroll(self.scroll_visual.get() / scale);
        }
    }

    /// Play the slide-and-fade that marks a new chapter.
    fn play_transition(&self, direction: Transition) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_enter_direction(match direction {
            Transition::Forward => 1.0,
            Transition::Backward => -1.0,
        });
        ui.set_enter_progress(0.0);

        let weak = self.me.clone();
        let start = std::time::Instant::now();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                let Some(app) = weak.upgrade() else { return };
                let Some(ui) = app.ui.upgrade() else { return };
                let elapsed = start.elapsed().as_secs_f32();
                let t = (elapsed / 0.28).clamp(0.0, 1.0);
                // Cubic ease-out.
                let eased = 1.0 - (1.0 - t).powi(3);
                ui.set_enter_progress(eased);
                if t >= 1.0 {
                    app.transition_timer.borrow_mut().take();
                }
            },
        );
        *self.transition_timer.borrow_mut() = Some(timer);
    }

    /// Paint a frame that came back from the worker.
    fn show_frame(&self, frame: Box<Frame>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let scale = self.scale.get().max(0.1);

        if let Some(band) = &frame.band {
            ui.set_band_image(bridge::image_from_rgba(&band.rgba, band.width, band.height));
            ui.set_band_height(band.height as f32 / scale);
            ui.set_band_width(band.width as f32 / scale);
        }
        if let Some(links) = &frame.links {
            *self.links.borrow_mut() = links.clone();
        }
        ui.set_band_top(frame.band_top / scale);
        ui.set_chapter_height(frame.chapter_height / scale);
        ui.set_viewport_fraction(
            (frame.viewport_height / frame.chapter_height.max(1.0)).clamp(0.05, 1.0),
        );
        ui.set_progress(frame.progress);
        ui.set_chapter_progress(frame.chapter_progress);
        ui.set_minutes_left(frame.minutes_left.unwrap_or(0) as i32);
        ui.set_chapter_label(frame.chapter_label.as_str().into());
        ui.set_focus_mode(frame.focus);
        ui.set_spotlight(frame.spotlight);
        ui.set_can_go_back(frame.can_go_back);
        ui.set_font_px(frame.font_px.round() as i32);
        ui.set_toc(bridge::toc_rows(&self.toc.borrow(), frame.toc_index));
        ui.set_toc_index(frame.toc_index.map(|i| i as i32).unwrap_or(-1));
        self.settings.borrow_mut().type_settings.font_px = frame.font_px;
        self.chapter.set(frame.chapter);

        // A new chapter arrives in place; the transition animates it in, so the
        // scroll offset must not also glide across the seam.
        match frame.transition {
            Some(direction) => {
                self.set_scroll(frame.scroll, true);
                self.play_transition(direction);
            }
            None => self.set_scroll(frame.scroll, false),
        }
        *self.frame.borrow_mut() = Some(frame);
    }
}

/// Open the window and run until it closes.
///
/// `book` is a path to open straight away; `data_root` overrides where state is
/// kept, which Android needs because an app's storage is handed to it rather
/// than found in the environment.
pub fn run(
    book: Option<PathBuf>,
    data_root: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_log_guard, _log_dir) = logging::init();

    let store = match &data_root {
        Some(root) => Store::rooted_at(root).ok(),
        None => Store::new().ok(),
    };
    let settings = store
        .as_ref()
        .map(|store| store.load_settings())
        .unwrap_or_default();
    let recents = store
        .as_ref()
        .map(|store| store.load_recents())
        .unwrap_or_default();

    let ui = AppWindow::new()?;
    // Wayland and X11 match the window to folio.desktop through this; it has to
    // be set after the platform exists but before the window is shown.
    if let Err(error) = slint::set_xdg_app_id("folio") {
        tracing::warn!(%error, "could not set the XDG app id");
    }
    let preset = settings.theme.resolve(ui.get_system_dark());

    let app = Rc::new_cyclic(|me: &std::rc::Weak<App>| App {
        me: me.clone(),
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
        links: RefCell::new(Vec::new()),
        scroll_target: Cell::new(0.0),
        scroll_visual: Cell::new(0.0),
        scroll_timer: RefCell::new(None),
        transition_timer: RefCell::new(None),
        chapter: Cell::new(0),
        #[cfg(target_os = "android")]
        books_dir: data_root.clone(),
        toc: RefCell::new(Vec::new()),
        families: RefCell::new(Vec::new()),
        notice_timer: RefCell::new(None),
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
    ui.set_justify(matches!(
        settings.type_settings.alignment,
        folio_core::layout::Alignment::Justified
    ));
    ui.set_measure(settings.type_settings.measure_ch.round() as i32);
    ui.set_follow_system(matches!(settings.theme, ThemeChoice::System));
    app.worker.send(Job::Fonts);

    if let Some(path) = book {
        app.open(path);
    }

    ui.run()?;
    app.save_settings();
    // Wait for the worker: it may still be holding an unwritten position.
    if let Ok(app) = Rc::try_unwrap(app) {
        app.worker.shutdown();
    }
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

    on!(on_scroll_by, |app: Rc<App>| move |delta: f32| {
        // Slint gives logical pixels; the reader thinks in physical ones.
        app.command(Command::Scroll(delta * app.scale.get()));
    });
    on!(on_scroll_screens, |app: Rc<App>| move |screens: i32| {
        app.command(Command::ScrollScreens(screens as f32));
    });
    on!(on_scroll_steps, |app: Rc<App>| move |steps: i32| {
        app.command(Command::ScrollSteps(steps as f32));
    });
    on!(on_scroll_to, |app: Rc<App>| move |fraction: f32| {
        app.command(Command::ScrollTo(fraction));
    });
    on!(on_jump_chapter, |app: Rc<App>| move |delta: i32| {
        if delta == 0 {
            // Home: back to the top of the chapter being read.
            app.command(Command::GoTo(Position::chapter_start(app.chapter.get())));
        } else {
            // Sections, not spine documents: the stepper is labelled with the
            // table-of-contents entry it is sitting in.
            app.command(Command::Section(delta));
        }
    });
    on!(on_toggle_focus, |app: Rc<App>| move || {
        let focus = app
            .frame
            .borrow()
            .as_ref()
            .map(|frame| frame.focus)
            .unwrap_or(false);
        if !focus {
            // Focus mode hides every control, including any hint of how to
            // leave it.
            app.notice("Focus mode — press Esc or f to leave");
        }
        app.command(Command::SetFocus(!focus));
    });
    on!(on_toggle_spotlight, |app: Rc<App>| move || {
        let spotlight = !app.settings.borrow().spotlight;
        app.settings.borrow_mut().spotlight = spotlight;
        app.command(Command::SetSpotlight(spotlight));
        let focused = app
            .frame
            .borrow()
            .as_ref()
            .map(|frame| frame.focus)
            .unwrap_or(false);
        // The spotlight only draws in focus mode, so toggling it while reading
        // normally changed nothing visible and looked broken.
        if spotlight && !focused {
            app.command(Command::SetFocus(true));
            app.notice("Spotlight on — press Esc or f to leave focus mode");
        } else if !spotlight {
            app.notice("Spotlight off");
        }
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
        app.pick_book();
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
    on!(on_pick_family, |app: Rc<App>| move |index: i32| {
        let family = app.families.borrow().get(index.max(0) as usize).cloned();
        if let Some(family) = family {
            app.edit_type_settings(move |settings| settings.family = Some(family));
        }
    });
    on!(on_set_justify, |app: Rc<App>| move |justify: bool| {
        app.edit_type_settings(move |settings| {
            settings.alignment = if justify {
                folio_core::layout::Alignment::Justified
            } else {
                folio_core::layout::Alignment::Left
            };
        });
    });
    on!(on_set_measure, |app: Rc<App>| move |measure: i32| {
        let measure = measure.clamp(40, 110) as f32;
        app.edit_type_settings(move |settings| settings.measure_ch = measure);
    });
    on!(on_pick_theme, |app: Rc<App>| move |index: i32| {
        let preset = match index {
            1 => ThemePreset::Sepia,
            2 => ThemePreset::Dark,
            _ => ThemePreset::Light,
        };
        app.theme.set(ThemeChoice::Fixed(preset));
        if let Some(ui) = app.ui.upgrade() {
            ui.set_follow_system(false);
        }
        app.apply_theme(preset);
    });
    on!(on_set_follow_system, |app: Rc<App>| move |follow: bool| {
        let (choice, preset) = if follow {
            let dark = app
                .ui
                .upgrade()
                .map(|ui| ui.get_system_dark())
                .unwrap_or(false);
            (ThemeChoice::System, ThemeChoice::System.resolve(dark))
        } else {
            let preset = app.preset.get();
            (ThemeChoice::Fixed(preset), preset)
        };
        app.theme.set(choice);
        if let Some(ui) = app.ui.upgrade() {
            ui.set_follow_system(follow);
        }
        app.apply_theme(preset);
    });
    on!(on_close_book, |app: Rc<App>| move || {
        app.close_book();
    });
    on!(on_quit, |app: Rc<App>| move || {
        app.save_settings();
        app.worker.send(Job::Flush);
        let _ = slint::quit_event_loop();
    });
    // (the event loop returning is what joins the worker, in main)
}

fn handle_event(app: &Rc<App>, event: Event) {
    // Frames produced for a previously open book must not overwrite the current
    // one; the worker and the UI agree on monotonic generations. The font list
    // belongs to no book, so it is never stale.
    let generation = match &event {
        Event::Opened { generation, .. }
        | Event::Frame { generation, .. }
        | Event::SearchDone { generation, .. }
        | Event::Failed { generation, .. } => *generation,
        // Neither belongs to a particular book.
        Event::Fonts { .. } | Event::Closed { .. } => u64::MAX,
    };
    if generation < app.book_generation.get() {
        tracing::debug!(generation, "dropping stale worker event");
        return;
    }

    match event {
        Event::Opened {
            title,
            author,
            toc,
            recents,
            ..
        } => {
            *app.toc.borrow_mut() = toc;
            *app.recents.borrow_mut() = recents.clone();
            app.has_book.set(true);
            if let Some(ui) = app.ui.upgrade() {
                ui.set_book_title(title.as_str().into());
                ui.set_book_author(author.as_str().into());
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
        Event::Closed { recents } => {
            // The shelf shows where each book was left, so it is refreshed
            // from the store once the position has been written.
            if let Some(ui) = app.ui.upgrade()
                && let Some(store) = &app.store
            {
                ui.set_shelf(bridge::shelf_rows(&recents, &store.covers_dir()));
            }
            *app.recents.borrow_mut() = recents;
        }
        Event::Fonts { families, current } => {
            if let Some(ui) = app.ui.upgrade() {
                let rows: Vec<slint::SharedString> =
                    families.iter().map(slint::SharedString::from).collect();
                ui.set_families(slint::ModelRc::new(slint::VecModel::from(rows)));
                *app.families.borrow_mut() = families;
                // Show the family in use: the chosen one when it is installed,
                // the resolved fallback otherwise.
                let chosen = app.settings.borrow().type_settings.family.clone();
                let shown = chosen
                    .filter(|name| app.family_index(Some(name)) >= 0)
                    .or(current);
                ui.set_family_index(app.family_index(shown.as_deref()));
            }
        }
        Event::Failed { message, .. } => {
            tracing::error!(%message, "command failed");
            app.notice(&message);
        }
    }
}

/// Android's entry point: the platform hands the app its window and its
/// storage, so there is nothing to find in the environment.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    let books = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .unwrap_or_else(|| PathBuf::from("/sdcard/Folio"));
    let _ = std::fs::create_dir_all(&books);

    if let Err(error) = slint::android::init(app) {
        eprintln!("could not start the Android backend: {error}");
        return;
    }
    if let Err(error) = run(None, Some(books)) {
        eprintln!("folio stopped: {error}");
    }
}
