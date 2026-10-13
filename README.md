# Folio

A quiet EPUB reader for the desktop, written in Rust with a [Slint](https://slint.dev) UI.

Folio ignores the typography publishers ship and imposes its own: one column,
one measure, one type scale, three themes. Books look like each other, and like
the rest of your reading, instead of like whatever CSS happened to be in the zip.

- **Continuous scrolling, one seam.** Reading inside a chapter is an unbroken
  scroll; the renderer paints a band taller than the window, so scrolling slides
  pixels rather than repainting them. Crossing into the next chapter is the only
  discontinuity, and it slides and fades in instead of snapping.
- **Switchable themes.** Light, Sepia, Dark, or whatever the desktop is doing.
  Because publisher CSS is discarded, no book can hard-code black-on-white and
  break dark mode.
- **Zoom that reflows.** Zoom changes the font size and re-flows the column,
  anchored on the line you were reading. Text never runs off the side.
- **Focus mode.** Hides every piece of chrome and narrows the measure; with the
  spotlight on, all but the paragraph you are reading is dimmed.
- Table of contents, in-book search, clickable footnotes and cross-references
  with a back stack, a recents shelf with covers, and your place remembered per
  book.

## Install

```sh
just install          # builds release, installs to ~/.local, associates .epub
```

Or by hand:

```sh
cargo build --release
install -Dm755 target/release/folio ~/.local/bin/folio
install -Dm644 packaging/folio.desktop ~/.local/share/applications/folio.desktop
install -Dm644 packaging/folio.svg ~/.local/share/icons/hicolor/scalable/apps/folio.svg
```

Build requirements are a Rust toolchain and the usual Linux UI libraries:
fontconfig, libxkbcommon, Wayland or X11, and EGL/GL. An AppImage script
(`just appimage`) and a Flatpak manifest (`packaging/flatpak/`) are included.

```sh
folio                      # opens the shelf
folio ~/books/moby.epub    # opens a book
```

## Keys

| Key | Action |
| --- | --- |
| `space` `PgDn` `→` | Scroll a screen (past the end: next chapter) |
| `PgUp` `←` | Scroll back a screen (past the top: previous chapter) |
| `↓` / `↑` | Scroll a step · mouse wheel scrolls too |
| `n` / `p` | Next / previous chapter · `Home` top of chapter |
| `f` | Focus mode · `Esc` leaves |
| `s` | Spotlight (dim all but the current paragraph, in focus mode) |
| `j` / `k` | Move the spotlight, or scroll when it is off |
| `t` | Cycle theme |
| `Ctrl` `+` / `-` / `0` | Zoom in, out, reset · `Ctrl`+scroll works too |
| `c` | Contents · `/` search · `Backspace` back |
| `?` | Keyboard shortcuts |
| `Esc` | Close the panel, then focus mode, then back to the shelf |
| `Ctrl`+`O` | Open a book · `Ctrl`+`W` put it away |

Clicking a link follows it; external links open in your browser. The status bar
carries a chapter stepper and the scrollbar can be dragged.

## How it works

Slint has no HTML engine, so Folio does the typography itself.

```
EPUB (rbook)                    crates/folio-core/src/book/
  └ XHTML ──html5ever──▶ Block/Span model        content/
                └──cosmic-text──▶ shaped lines   layout/engine.rs
                        └──flow──▶ one column    layout/flow.rs
                              └──band raster──▶ RGBA  layout/raster.rs
                                    └──▶ Slint Image, slid by scroll offset
```

- `crates/folio-core` has **no Slint dependency** — parsing, layout, pagination,
  raster, the reading cursor and persistence all live there and are tested
  headlessly. `crates/folio` is the window.
- A worker thread owns the reader, the font system and the caches; the UI thread
  only sends commands and receives frames, so a long chapter can never stall the
  window. Jobs are coalesced, so held keys skip intermediate frames.
- Laid-out chapters are cached (current plus one each side, prefetched). Layout
  is the expensive step and raster is cheap, which is why a theme switch or a
  spotlight move repaints without re-shaping. A band covers about two screens
  and is capped at 32MB, so scrolling only asks for new pixels near its edges.
- Reading positions are stored as `(chapter, block, character offset)`, never as
  scroll offsets, so they survive zoom, resize and font changes.

State lives in `~/.config/folio/settings.json`, `~/.local/share/folio/` (recents
and covers) and `~/.local/state/folio/logs/`. Run with `RUST_LOG=folio=debug`
for detail.

## Known limitations

- **No text selection or copy.** The page is a painted image and selection is a
  real model (cross-block ranges, drag states); it was deliberately left out.
- **No hyphenation.** cosmic-text breaks at soft hyphens without drawing one, so
  justification is available (opt-in) but ragged right is the default.
- **Publisher CSS is ignored by design**, including inline styles. Books that are
  really page layouts (fixed-layout comics, heavily designed cookbooks) will look
  like plain prose.
- **Tables are flattened** into paragraphs, one per cell.
- **No two-page or paginated mode.** Reading is a scroll; there are no page
  numbers, only a position within the chapter and the book.
- Fonts come from the system; nothing is bundled, so the reading face depends on
  what is installed (the preference chain starts at Charis SIL and Literata and
  falls back to Noto Serif, DejaVu Serif, Liberation Serif).
- Drag-and-drop onto the window is not wired up; use `Ctrl`+`O` or the shelf.

## Development

```sh
just test     # cargo test --workspace
just lint     # fmt --check + clippy -D warnings
just page ~/books/moby.epub "--chapter 3 --screens 2 --theme dark --focus"
```

That last one renders a page straight to a PNG with no window, which is the
quickest way to look at a layout change. For the whole window — chrome, panels
and all — there is a headless screenshot tool that uses Slint's software
renderer, so UI shots are reproducible and need no compositor:

```sh
cargo run -p folio --example ui_shot -- shelf shots/shelf.png 1100x820
cargo run -p folio --example ui_shot -- toc shots/toc.png 1100x820 ~/books/moby.epub
# scenes: shelf · reading · toc · search · settings · focus
# FOLIO_SHOT_THEME=light|sepia|dark picks the palette
``` Tests cover the XHTML walker (including
malformed markup), flow invariants as property tests (blocks stack in order and
never overlap; the line under the scroll offset round-trips at every zoom step),
the reading session including chapter crossings and band reuse, and the window's
key map through Slint's headless testing backend.

## License

MIT.
