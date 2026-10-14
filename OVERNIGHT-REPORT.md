# Overnight run — report

2026-10-14, 00:56 → 02:00 local. Three review passes (two UI, one correctness),
all findings triaged, the real ones fixed. Everything is committed and pushed to
`github.com/nabinthapaa/epub-reader` (private).

**State: 45 tests green, clippy clean at `-D warnings`, `cargo fmt` clean,
`~/.local/bin/folio` refreshed.**

---

## What happened, in order

| Commit | What |
|---|---|
| `2710f30` | UI round 1 — the eight "fix now" findings |
| `d841d5f` | Draggable scrollbar, contents follows the reader |
| `131f3c1` | Honest focus-mode screenshots, one label style |
| `ff86888` | UI round 2 — balance, the light theme, chapter stepper |
| `55bd4dc` | Tests for the new reading maths and the shortcuts card |
| `f1df856` | Round 2 recorded; README updated |
| `c5501c6` | Everything the correctness review found |

Screenshots: `.overnight/shots-round-0/` (before), `shots-round-1/`,
`shots-round-2/`, `shots-final/` (now, including sepia and the shortcuts card).
All rendered headlessly — no window was opened while you slept.

## The three reviews

**UI round 1** found 17 items. Two of its claims were false and checking them
paid for the exercise:

- "The body font changes between scenes" — it does not; both resolve to Noto
  Serif. What it had actually seen was the band being **stretched**: the Image
  was bound to `parent.width` with `image-fit: fill`, so any band that had not
  caught up with a panel opening or a resize was squashed sideways. Real bug,
  wrong diagnosis.
- "Focus mode doesn't narrow the measure" — it does. My screenshot tool was
  setting the UI flag without telling the reader, so the band was rasterized at
  the full measure. The tool was lying, not the app.

**UI round 2** measured the chrome rather than eyeballing it, and caught a
regression I had just introduced: swapping ghosted controls for
`visible: has-book` traded one problem for another, because Slint's
`visible: false` hides an element but keeps its layout space — the toolbar's
right cluster sat 75px from the edge against the left's 18px. It also found a
bug present since the search panel was written: its divider had a width but no
`x`, and Slint centres a child positioned that way, so the border ran down the
middle of the panel. Verdict: shippable.

**The correctness review** said **not yet**, and it was right. Two defects
reproduced as panics, one as wrong numbers.

## The bugs that mattered

1. **Coalescing dropped painted bands.** The reader rasterizes on every apply
   and records "this is what the window shows"; the worker then decided whether
   to ship it. A discarded frame left the reader believing in pixels the UI
   never received, so the next frame legitimately said "no new band" and the
   window drew the *previous* band at the *new* band's origin — content jumping
   half a screen during fast scrolling, which is the app's most common
   interaction. Intermediate commands now move the cursor without rasterizing.
2. **The band's byte cap could shrink it below the viewport plus margins**,
   so on a 3440×1440 ultrawide every wheel tick repainted 32MB and the band's
   whole reason for existing was silently gone. The floor is hard now; a
   table-driven test asserts it from 1280×800 to 5K, where it used to fail.
3. **Space/PageDown showed a strip of bare page** for a few frames: the UI eased
   jumps of up to a full viewport while the band carries only 0.6 of one as
   pre-roll. The snap threshold is derived from the pre-roll now.
4. **Quitting could lose your place.** The worker thread was detached, so the
   process could exit while it still held an unwritten position. It is joined.
5. **Search could panic the worker** on any book containing `Å` (U+212B), `İ`
   or similar: case folding is not length-preserving, and the code mixed byte
   offsets of the original text with offsets into its lowercased copy. And a
   dead worker left a live window that answered nothing — it now says so.
6. **Progress drifted and could go backwards**, because it recomputed its own
   denominator as chapters were parsed — and that number is persisted to the
   shelf. Weights are taken once at open from spine resource sizes. It also
   counted bytes while calling them characters, over-reporting "minutes left"
   by roughly double on any non-Latin book.
7. **One new field would have wiped your shelf.** `RecentBook` had no
   `#[serde(default)]`, so adding a field would make the file unparseable, and
   the next write would replace the whole shelf with a single entry. Both
   structs tolerate missing fields now, settings are clamped on load, and an
   unreadable shelf is left alone instead of truncated.

Worth noting: the `#[serde(default)]` fix silently did nothing on my first
attempt — I edited a struct definition in the wrong file and the string replace
matched nothing. The test I had just written caught it immediately.

## What I deliberately did not do

- **Panels still inset the reading area rather than floating over it.** Opening
  Contents shifts the column ~133px sideways. Overlaying would nail the prose
  in place but cover 25–65px of it at this window size and more on a narrow
  one. Insetting never hides text; the shift is the honest cost. Your call if
  it grates.
- **The shelf is still one horizontally-scrolling row**, not a wrapping grid.
  Slint has no wrap layout, so this needs either chunking the model in Rust by
  window width or a hand-rolled grid. Worth doing when the shelf outgrows a
  screen.
- **Search is still synchronous on the worker** and walks every chapter with no
  cancellation. On Moby Dick it is fast enough; on a very large book it would
  block rendering for a moment.
- Non-goals from the original design stay non-goals: text selection, hyphenation,
  publisher CSS, a paginated mode, a folder-scanning library.

## What needs your decision

1. **Panels: inset or overlay?** Above. One afternoon either way.
2. **The ease constants.** Scrolling eases at 28% per 16ms tick and the chapter
   transition is 280ms. Both are one-line changes in `crates/folio/src/main.rs`.
   I still have not felt either — I cannot drive the GUI from here.
3. **Progress weighting.** It now uses compressed spine-resource sizes, which is
   stable and roughly proportional to text, but a chapter that is mostly images
   will weigh more than it reads. The alternative is parsing the whole book at
   open (about a second for Moby Dick).
4. **Repo visibility.** Still private, as you chose. `gh repo edit --visibility public`
   when you want it public.

## Honest state

- `cargo test --workspace`: **45 passed, 0 failed** (8 unit, 3 band, 5 flow, 8
  parse, 15 reading, 5 store, 1 UI scenario bundle).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all --check`: clean.
- Untested by anything: the worker's own protocol (the coalescing fix is
  exercised only indirectly), the Slint geometry beyond the key map, and
  anything that needs a real window.
- The code reviewer's remaining nits — a duplicate font-database scan at
  startup, `ui_shot` not correcting the band height for chrome, `active_toc`
  assuming TOC order matches spine order — are listed in its report and were
  judged not worth the churn tonight.
