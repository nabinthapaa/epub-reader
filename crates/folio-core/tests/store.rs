//! Persistence. A reader that forgets where you were has lost the only state
//! it has, so these are about never losing it — including when a future
//! version adds a field.

use folio_core::state::{Position, RecentBook, Settings};
use folio_core::store::Store;

#[test]
fn settings_survive_a_file_written_by_an_older_version() {
    // Settings flattens TypeSettings, and a container default on the outer
    // struct does not cover a flattened field: without #[serde(default)] on
    // TypeSettings itself, this fails to parse and silently resets everything.
    let older = r#"{"theme":{"mode":"fixed","preset":"dark"},"toc_open":true}"#;
    let settings: Settings = serde_json::from_str(older).expect("an older file still parses");
    assert!(settings.toc_open, "the fields that were there are kept");
    assert_eq!(settings.type_settings.font_px, 18.0, "the rest default");
}

#[test]
fn recents_survive_an_entry_from_an_older_version() {
    let older = r#"[{"path":"/books/a.epub","title":"A","position":{"chapter":3,"block":7}}]"#;
    let recents: Vec<RecentBook> = serde_json::from_str(older).expect("older entries still parse");
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].position.chapter, 3);
    assert_eq!(recents[0].author, "");
}

#[test]
fn settings_round_trip_through_json() {
    let mut settings = Settings::default();
    settings.type_settings.font_px = 28.0;
    settings.spotlight = true;
    settings.window_width = 1234;

    let json = serde_json::to_string(&settings).unwrap();
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(back, settings);
}

#[test]
fn nonsense_settings_are_clamped_rather_than_handed_to_the_layout_engine() {
    // cosmic-text asserts on a zero line height, on the render thread, where a
    // panic freezes the window with no message.
    let json = r#"{"font_px":0.0,"line_height":0.0,"measure_ch":4000.0,"margin_px":-5.0}"#;
    let parsed: Settings = serde_json::from_str(json).unwrap();
    let safe = parsed.sanitize();

    assert!(safe.type_settings.font_px >= 12.0);
    assert!(safe.type_settings.line_height >= 1.0);
    assert!(safe.type_settings.measure_ch <= 110.0);
    assert!(safe.type_settings.margin_px >= 0.0);
}

#[test]
fn a_position_written_to_the_shelf_comes_back() {
    // Uses the real store, so it touches the user's own directories only in
    // the sense that it writes an entry under a path that cannot exist.
    let Ok(store) = Store::new() else {
        return; // No writable config dir (CI sandbox): nothing to assert.
    };
    let path = format!(
        "/nonexistent/folio-test-{}.epub",
        folio_core::store::now_secs()
    );
    let entry = RecentBook {
        path: path.clone(),
        title: "Test".into(),
        author: "A. Tester".into(),
        position: Position {
            chapter: 4,
            block: 11,
            char_offset: 3,
        },
        progress: 0.42,
        cover: None,
        opened_at: folio_core::store::now_secs(),
    };

    store.touch_recent(entry).expect("the shelf is writable");
    assert_eq!(
        store.position_for(&path),
        Some(Position {
            chapter: 4,
            block: 11,
            char_offset: 3
        })
    );

    store
        .update_position(
            &path,
            Position {
                chapter: 9,
                block: 2,
                char_offset: 0,
            },
            0.9,
        )
        .expect("updating works");
    assert_eq!(store.position_for(&path).map(|p| p.chapter), Some(9));

    // Clean up after ourselves: the shelf belongs to the user.
    let remaining: Vec<RecentBook> = store
        .load_recents()
        .into_iter()
        .filter(|book| book.path != path)
        .collect();
    store.save_recents(&remaining).unwrap();
    assert_eq!(store.position_for(&path), None);
}
