# Folio — EPUB reader (Rust + Slint)

A desktop EPUB reader: simple, themed (light/sepia/dark/system), reflowing zoom,
focus mode with paragraph spotlight.

## Decisions (settled in design interview, 2026-10-13)

| Area | Decision |
|---|---|
| Content rendering | Parse XHTML → `Block`/`Span` model → lay out with **cosmic-text** → rasterize to RGBA → show in Slint as an `Image`. Chrome is native Slint widgets. |
| Reading model | **Paginated**. Chapter laid out at viewport width, lines sliced into pages, only the current page rasterized. Position = (chapter, page) for display, logical anchor for storage. |
| Publisher CSS | **Ignored**, including inline `style`. Only a semantic-tag whitelist is honored. Our theme and type scale always win. |
| Container layer | **rbook 0.7.12** behind our own `Book` trait (`folio-core::book::ports`). Only `rbook_book.rs` names rbook. |
| XHTML parsing | **html5ever** full tree construction (+ `markup5ever_rcdom`), recursive DOM walk with an inline style stack. Never fails on broken markup. |
| Threading | **Worker thread** owns `FontSystem`, layouts and raster. Request/response with a generation counter; stale responses dropped. |
| Caching | **Layout LRU of 3 chapters** (prefetch prev/next chapter); pixmap LRU ≤3 pages under a byte budget. Layout key `(chapter, width, font_px, margins)`; pixmap key adds `(page, theme, focus)`. Theme/focus changes re-raster only. |
| Theme | Tokens live in a Rust `Theme` struct; exported into Slint as a struct. Presets: Light, Sepia, Dark, Follow system (`Palette.color-scheme`). |
| Zoom | Base **font size in px**, discrete steps `12 14 16 18 20 24 28 34 40 48`, relayout + repaginate, anchored on the first visible line. No horizontal scrolling ever. `Ctrl+=` `Ctrl+-` `Ctrl+0` `Ctrl+scroll`. |
| Focus mode | `F` hides all chrome and narrows the measure (70ch → 62ch); `Esc` exits. Optional **spotlight**: non-active paragraphs at 35% alpha, active block advanced with `j`/`k`/`↓`/`↑`. Alpha per block at raster time, no relayout. |
| Fonts | **System fonts only** via fontconfig. Preferred-family chain picks the first installed (Charis SIL → Literata → Noto Serif → DejaVu Serif → Liberation Serif → generic serif); settings lists installed families. Synthetic oblique/embolden when a face is missing. |
| Typography | Ragged right by default, **justify opt-in** (`Align::Justified`). No hyphenation (cosmic-text breaks at U+00AD without drawing a hyphen). Measure `min(viewport − 2×margin, 70ch)`, line height 1.6. |
| Images | Raster via `image` (png/jpeg/gif/webp) + SVG via `resvg`. Fit to column, centered, caption below. Taller than remaining space → next page; taller than a page → scaled to its own page. Missing/corrupt → alt text in a dashed frame. |
| Interaction | Span hit-testing for links: internal `href#anchor` → position anchor + back-stack (`Backspace`), external → system browser. In-book search (`/`) over cached per-chapter text on the worker, results jump to an anchor. **No text selection in v1.** |
| Position model | `Position { spine, block, char }` — independent of width/font, survives zoom and resize. `record ∘ resolve == identity`. |
| Persistence | `~/.config/folio/settings.json` (theme, font family, font_px, justify, margins, spotlight); `~/.local/share/folio/recents.json` (≤24 entries, path → Position + progress); `~/.local/share/folio/covers/<hash>.png`. Atomic writes (tmp + rename), ~1s debounce, flush on exit. |
| Shelf | Recents shelf with cover tiles and progress; open via CLI arg, file dialog (`rfd`), drag-and-drop. No folder scanning. |
| Tests | Golden tests for XHTML→blocks (incl. malformed); proptests for pagination + anchor round-trip; headless UI smoke tests via `i-slint-backend-testing`. EPUB fixtures generated in code as zip bytes — no binary blobs committed. **No pixel snapshots** (system fonts make them machine-dependent). |
| Structure | Workspace: `crates/folio-core` (**no Slint dependency**) + `crates/folio` (binary, Slint, rfd). |
| Backend | Default Slint backend (winit + femtovg); `SLINT_BACKEND=software` fallback. Skia not enabled (build cost, no benefit for one uploaded image). |
| Hardening | `tracing` + rolling file log + panic hook with error dialog; `folio.desktop` with `MimeType=application/epub+zip`, icon, `just install`; GitHub Actions (fmt, clippy -D warnings, tests, tagged release); AppImage + Flatpak manifest (Flatpak needs `rfd` XDG portal backend). |

## Layout

```
folio/
  Cargo.toml                  workspace
  crates/folio-core/          no slint
    src/book/{ports,rbook_book,resources}.rs
    src/content/{blocks,html}.rs
    src/layout/{engine,paginate,raster,measure}.rs
    src/{theme,state,store,search,error}.rs
  crates/folio/               binary
    build.rs                  slint-build
    ui/{app,reader,shelf,toolbar,toc,settings,tokens}.slint
    src/{main,worker,shortcuts,bridge,logging}.rs
  packaging/{folio.desktop,folio.svg,flatpak,appimage}
  .github/workflows/{ci,release}.yml
```

## Milestones

1. **M1 vertical slice** — open an EPUB, parse a chapter, lay out, paginate, raster, show in a Slint window, turn pages with ←/→. Proves rbook + html5ever + cosmic-text + worker + Slint image upload end to end.
2. **M2 chrome** — toolbar, TOC panel, status bar (chapter · page x/y · progress), keyboard map.
3. **M3 themes** — Theme struct → Slint + rasterizer; Light/Sepia/Dark/system; re-raster without relayout.
4. **M4 zoom** — discrete steps, anchored repagination, Ctrl+scroll, reset.
5. **M5 focus mode** — chrome hide, narrowed measure, spotlight with j/k.
6. **M6 links + search** — hit-testing, back-stack, external links, in-book search UI.
7. **M7 shelf + persistence** — recents, covers, settings and position round-trip, drag-and-drop, file dialog.
8. **M8 hardening** — logging, panic dialog, desktop integration, install target, CI, AppImage + Flatpak.

Each milestone is verified by launching the app on real public-domain EPUBs
(downloaded to a cache dir, never committed) and screenshotting it.
