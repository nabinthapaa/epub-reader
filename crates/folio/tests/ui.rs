//! Headless checks of the window itself: that keys reach the right commands and
//! that the panels open and close as the reader expects.
//!
//! These run against the real .slint UI with Slint's testing backend, so a
//! broken key map or an unwired callback fails here rather than in someone's
//! hands.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};

slint::include_modules!();

/// Every test runs against Slint's headless backend, initialized once.
fn init_backend() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(i_slint_backend_testing::init_no_event_loop);
}

/// Records which callbacks the UI invoked, in order.
type Log = Rc<RefCell<Vec<String>>>;

fn app() -> (AppWindow, Log) {
    init_backend();
    let ui = AppWindow::new().expect("the window builds");
    let log: Log = Rc::new(RefCell::new(Vec::new()));

    ui.set_has_book(true);
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));

    macro_rules! record {
        ($setter:ident, $name:literal) => {{
            let log = Rc::clone(&log);
            ui.$setter(move || log.borrow_mut().push($name.to_string()));
        }};
        ($setter:ident, $name:literal, int) => {{
            let log = Rc::clone(&log);
            ui.$setter(move |value: i32| log.borrow_mut().push(format!("{}({})", $name, value)));
        }};
    }

    record!(on_scroll_screens, "scroll-screens", int);
    record!(on_scroll_steps, "scroll-steps", int);
    record!(on_jump_chapter, "jump-chapter", int);
    record!(on_move_spotlight, "move-spotlight", int);
    record!(on_zoom, "zoom", int);
    record!(on_zoom_reset, "zoom-reset");
    record!(on_toggle_focus, "toggle-focus");
    record!(on_toggle_spotlight, "toggle-spotlight");
    record!(on_cycle_theme, "cycle-theme");
    record!(on_back, "back");
    record!(on_open_file, "open-file");

    ui.invoke_focus_reader();
    (ui, log)
}

fn press(ui: &AppWindow, key: impl Into<slint::SharedString>) {
    let text = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn press_with_control(ui: &AppWindow, key: &str) {
    let control: slint::SharedString = Key::Control.into();
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: control.clone(),
    });
    press(ui, key);
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: control });
}

fn taken(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// One test, because Slint windows belong to a single thread and cargo runs
/// test functions in parallel. Each block below is an independent scenario.
#[test]
fn the_window_responds_to_keys_and_panels() {
    // --- arrows, space and page keys scroll ---
    {
        let (ui, log) = app();

        press(&ui, Key::RightArrow);
        press(&ui, Key::LeftArrow);
        press(&ui, " ");
        press(&ui, Key::PageDown);
        press(&ui, Key::PageUp);
        press(&ui, Key::DownArrow);
        press(&ui, Key::UpArrow);

        assert_eq!(
            taken(&log),
            vec![
                "scroll-screens(1)",
                "scroll-screens(-1)",
                "scroll-screens(1)",
                "scroll-screens(1)",
                "scroll-screens(-1)",
                "scroll-steps(1)",
                "scroll-steps(-1)",
            ]
        );
    }

    // --- view keys reach their commands ---
    {
        let (ui, log) = app();

        press(&ui, "f");
        press(&ui, "s");
        press(&ui, "t");
        press(&ui, Key::Backspace);
        press(&ui, "n");
        press(&ui, "p");

        assert_eq!(
            taken(&log),
            vec![
                "toggle-focus",
                "toggle-spotlight",
                "cycle-theme",
                "back",
                "jump-chapter(1)",
                "jump-chapter(-1)",
            ]
        );
    }

    // --- control shortcuts zoom and open ---
    {
        let (ui, log) = app();

        press_with_control(&ui, "=");
        press_with_control(&ui, "-");
        press_with_control(&ui, "0");
        press_with_control(&ui, "o");

        assert_eq!(
            taken(&log),
            vec!["zoom(1)", "zoom(-1)", "zoom-reset", "open-file"]
        );
    }

    // --- j and k move the spotlight only in focus mode ---
    {
        let (ui, log) = app();

        press(&ui, "j");
        assert_eq!(
            taken(&log),
            vec!["scroll-steps(1)"],
            "plain reading: j scrolls"
        );

        ui.set_focus_mode(true);
        ui.set_spotlight(true);
        press(&ui, "j");
        press(&ui, "k");
        assert_eq!(
            taken(&log),
            vec!["move-spotlight(1)", "move-spotlight(-1)"],
            "in focus mode with the spotlight on, j/k walk paragraphs"
        );
    }

    // --- escape closes panels innermost first ---
    {
        let (ui, log) = app();

        ui.set_toc_open(true);
        ui.set_search_open(true);

        press(&ui, Key::Escape);
        assert!(!ui.get_search_open(), "search closes first");
        assert!(ui.get_toc_open(), "contents stays open");

        ui.set_focus_mode(true);
        press(&ui, Key::Escape);
        assert_eq!(taken(&log), vec!["toggle-focus"], "focus mode exits next");

        ui.set_focus_mode(false);
        press(&ui, Key::Escape);
        assert!(!ui.get_toc_open(), "then the contents panel closes");
    }

    // --- panel keys toggle the side panels ---
    {
        let (ui, _log) = app();

        press(&ui, "c");
        assert!(ui.get_toc_open());
        press(&ui, "c");
        assert!(!ui.get_toc_open());

        press(&ui, "/");
        assert!(ui.get_search_open(), "slash opens search");
    }

    // --- the shelf is shown until a book is open ---
    {
        let (ui, log) = app();
        ui.set_has_book(false);

        // With no book, reading keys do nothing at all.
        press(&ui, Key::RightArrow);
        press(&ui, "f");
        assert!(taken(&log).is_empty());

        // Opening a file still works.
        press_with_control(&ui, "o");
        assert_eq!(taken(&log), vec!["open-file"]);
    }

    // --- status values render from the pushed state ---
    {
        let (ui, _log) = app();
        ui.set_progress(0.41);
        ui.set_chapter_progress(0.6);
        ui.set_chapter_label("CHAPTER 9. The Whale".into());
        ui.set_scroll(320.0);
        ui.set_band_top(100.0);

        assert_eq!(ui.get_chapter_label(), "CHAPTER 9. The Whale");
        assert_eq!(ui.get_chapter_progress(), 0.6);
        // The band slides by the difference: chapter space -> viewport space.
        assert_eq!(ui.get_band_top() - ui.get_scroll(), -220.0);
    }
}
