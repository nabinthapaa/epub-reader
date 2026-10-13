# UI review findings

Newest round at the top. Each round: implement the top items, re-render the
scenes with `ui_shot`, review again, prepend the new findings here.

---

## Round 2 — 2026-10-14, after the round-1 fixes

Measured from `.overnight/shots-round-1/`. The reviewer's verdict: **shippable**.
"The reading surface is the product and it is good." Everything below is done
unless marked otherwise.

### Done this round (commit ff86888)

1. **Toolbar balance** — round 1's `visible: has-book` hid controls but kept
   their layout space, so the right cluster sat 75px from the edge against the
   left's 18px. Every one is now an `if`. This was a regression round 1 caused.
2. **The search panel's divider ran down its middle** — a `Rectangle` with a
   width but no `x`, which Slint centres. Present since the panel was written.
3. **Light theme was never tuned**: the scrollbar track and focus hairline used
   text-alpha values chosen against a dark page and vanished on white. Both use
   `border` now, the thumb uses `muted`, and light prose went 16.2:1 → 13.2:1.
4. **"~83 min left" was chapter-scoped beside a book-scoped bar** — it now says
   "in chapter", and the status bar groups chapter-left, book-right.
5. **Chapter stepper** in the status bar's dead space: `n`/`p` were bound but
   had no visible affordance anywhere, which the reviewer called the single
   highest-value thing missing for a first-time user.
6. Settings popover alignment: typeface picker is a field with a chevron, line
   length is one segmented control, checkbox ink aligns, "?" is a key cap.
7. Shortcuts card has real columns (it faked them with spaces in a proportional
   font); focus mode letterboxes the page; contents reveals the current row
   through a function rather than a `viewport-y` binding the user's own
   scrolling would replace; links show a pointer; shelf titles elide past three
   lines; the shot tool scrolls into real prose.

### Deliberately not done

- **Panels still inset the reading area rather than floating over it** (round 2
  item 10). Opening Contents shifts the column ~133px sideways. Overlaying
  would nail the prose in place but cover 25–65px of it at this window size,
  and more on a narrow window. Insetting is what Apple Books and friends do and
  it never hides text; the shift is the honest cost. Revisit if it grates.
- The shelf is still one horizontally-scrolling row, not a wrapping grid: Slint
  has no wrap layout, so this needs either chunking the model in Rust by window
  width or a hand-rolled grid. Worth doing when the shelf holds more than a
  screen's worth of books.

### Corrections recorded against the reviewer

- Round 1's "the body font differs between scenes" was false: both resolve to
  Noto Serif. The real defect it pointed at was the band being stretched by
  `image-fit: fill` at `parent.width`.
- Round 1's measure-narrowing diagnosis blamed a flag only focus mode sets.
- Round 2's "focus mode is reading minus chrome" partly rested on a shot where
  the tool never told the reader about focus mode, so the band was rasterized
  at the full measure.


## Round 1 — 2026-10-13, dark/light at 1100x820

Reviewed: shelf, reading (dark + light), toc, search, settings, focus.
Measurements below were sampled from the PNGs, so treat them as real numbers.

### Verify before acting (my notes, not the reviewer's)

- **Item 6's mechanism is misdiagnosed.** `column_width(viewport_w, narrowed)`
  is called with `narrowed = self.focus` (`reader.rs::geometry`), so panels do
  **not** trigger the 0.88 measure. The re-wrap the reviewer saw between
  `reading.png` and `toc.png` is real but has a different cause: the reader area
  genuinely loses 260px when the panel insets it, so the column is narrower.
  The choice is therefore (a) accept the re-wrap, or (b) overlay the panels on
  top of the text instead of insetting them. **Do not delete the focus-mode
  narrowing** — it is a decided feature the user asked for.
- **The "different body face" claim** (also item 6) is unconfirmed. Check it by
  rendering two scenes with identical settings before changing anything; if the
  family really differs after a re-layout, that is a font-resolution bug and
  outranks everything else on this list.
- Item 4 proposes removing the theme button from the toolbar and hiding `Open`
  while reading. Reasonable, but it is a judgement call about the product, not a
  defect — implement only if the result reads better in the screenshots.

### Fix now

1. **Toolbar chips render as full-height 46px slabs, not 30px pills.**
   `ToolButton` pins 30px min/max height and a 6px radius, but inside the
   toolbar layout the active fill runs y=0..45 (sampled at x=743 in
   `search.png`), so corners are clipped and the active state reads as a bar of
   colour. Fix: `padding-top: 8px; padding-bottom: 8px` on the toolbar's
   `HorizontalLayout`. *Trivial.*

2. **The std-widgets controls ignore the theme.** Search field samples `#C0C1C2`
   and the typeface `ComboBox` `#BBBCBD`, both near-white on a `#1E2023`
   surface — in `search.png` the field is the brightest thing on screen,
   brighter than the prose. It contradicts the app's premise that the palette is
   ours. Replace `LineEdit`/`ComboBox`/`CheckBox` with token-driven primitives:
   a `Rectangle` + `TextInput` field (page background, border, 32px, muted
   placeholder), a `ToolButton` that opens a short family list, and a 16px
   checkbox square (border `tokens.border`, fill `tokens.accent` when checked).
   *Moderate, ~70 lines of Slint, no Rust.* **Biggest single credibility jump.**

3. **`/` opens search but never focuses the field.** `focus-field()` exists in
   `panels.slint` and has zero call sites. Name the panel instance and call it
   from the `/` branch, the `Ctrl+F` branch and the toolbar handler; make Escape
   inside the field return focus to the reader. *Trivial.*

4. **The toolbar is ten controls and four metaphors.** The title is centred in
   the stretch region, not the window (x≈429 against a 550 centre), because the
   right cluster is ~3× the left. Mixed glyph/emoji/word labels (`☰`, `🔍`, `⚙`,
   words). Ambiguous state labels (`Dark` — am I in dark or going dark?).
   `18px` is secretly "reset zoom" and nobody will find it. Six of ten controls
   are ghosted at 0.4 opacity on the shelf, where the toolbar title also
   duplicates the H1. Fix: absolutely position the title and centre it on the
   window, title in `tokens.text` + `" — " + author` in `tokens.muted`; group
   `A− 18px A+` into one segmented control; pick words over symbols; use
   `visible: has-book` instead of disabling on the shelf; drop the duplicate
   title there. *Trivial to moderate.*

5. **The status bar shows four position readouts, one wrong and one fictional.**
   "Chapter 2" is the fallback label while the screen shows the Gutenberg title
   page. "8%" is `(chapter_index + fraction) / chapter_count`, which is fiction
   for a book whose spine files vary wildly in length. None of them answers
   "how much longer?". Fix: fall back to the book title or "Front matter"
   instead of `Chapter {n}`; weight `progress()` by chapter length (lazily
   accumulate per-chapter character counts — the search path already walks every
   chapter); show "~14 min left in chapter" (chars / 5 / 250 wpm); drop "0% of
   chapter" since the scrollbar covers it. *Moderate, Rust side.*

6. **Opening a panel re-flows the prose.** Same paragraph breaks at different
   words between `reading.png` and `toc.png`. See my note above for the real
   cause and the two options. *Trivial either way.*

7. **The settings popover has no elevation.** `surface` on `page` is a 1.08
   luminance ratio with a barely visible 1px border and no shadow, so a heading
   is chopped mid-word by an edge you cannot see. The dismiss `TouchArea`
   exists but is invisible. Fix: visible scrim (`#000` @0.28 dark / @0.18
   light, animated 120ms) plus `drop-shadow-blur: 24px`, offset-y 8px. *Trivial.*

8. **The contents panel marks nothing as current.** Every row samples flat
   `#1E2023` — either `active_toc()` returns `None` in front matter or the flag
   is not reaching the model. It also never scrolls the current entry into view
   (painful in a 135-entry book), and the `ScrollView` track paints a second
   vertical line 15px inside the panel border. Fix: fall back to the nearest
   preceding entry so something is always current; scroll it to ~⅓ down on
   open; mark it with a 3px accent left bar (the 0.16 tint is invisible in
   dark); style or hide the scrollbar. *(a) moderate, rest trivial.*

### Nice later

9. **Nothing is discoverable.** Focus mode, spotlight, search, zoom reset,
   chapter jump and Escape-to-shelf are documented only in two 11px muted
   one-liners. Worse, `s` outside focus mode is a silent no-op, so it reads as
   broken. Fix: a `?` shortcuts card; fire the existing notice toast on entering
   focus mode ("Focus mode — press Esc or f to exit"); make `s` outside focus
   either enter focus with the spotlight on, or explain itself. *Card moderate,
   rest trivial.*

10. **The shelf is 60% empty, truncates every title, and scrolls sideways.**
    Content ends at y≈500 of 820. Titles elide to one line despite 32px (two
    lines) being reserved. Metadata mixes `"7% · Lewis Carroll"` with a bare
    author and `"3% · Unknown"`. Horizontal drag is an unusual gesture explained
    by a muted hint. Fix: wrapping grid in a vertical `ScrollView`; let titles
    use both reserved lines; author on its own line, no "Unknown"; progress as a
    3px accent bar on the cover's bottom edge (unread books simply have none);
    "Last read 3 days ago" if the timestamp is in the store; shrink the "+ Open
    file…" touch area to its visible box. *Moderate.*

11. **The scrollbar is a 1.4:1 hairline, chapter-scoped, not draggable.** No
    track, so chapter length is unreadable; no `TouchArea`, so it cannot be
    dragged; and it contradicts the book-level bar 20px below. Fix: 4px wide,
    radius 2, 0.26 resting / 0.5 hover, over a 0.06-alpha track; 14px hit strip
    mapping y → scroll. *Visuals trivial; drag needs a `scroll-to(fraction)`
    command in `reader.rs`.*

12. **Dark glares slightly and its chrome barely separates.** `#DCDAD4` on
    `#161719` is ≈12.9:1 — fine for UI, hot for an hour of prose — and
    `surface` vs `page` is a 1.08 ratio held apart only by a hairline. Proposed
    (Dark only): `page #1A1C1F`, `surface #24262A`, `border #383C42`,
    `text #CDCAC3` (≈9.8:1); keep the accent. Light needs nothing. *Trivial, but
    re-check the spotlight `dim_factor` after.*

13. **Lines are sliced in half at the viewport edges** ("MOBY-DICK;" cut by the
    status bar). Add non-interactive gradient fades: 28px at the bottom, 20px at
    the top, `tokens.page → transparent`. *Trivial, high perceived quality.*

14. **The focus-mode progress hairline reads as a glitch** — an 85px blue stub
    in the corner at 8%. Give it a full-width 0.07-alpha track, or make it
    `tokens.muted` @0.45. *Trivial.*

15. **Chrome typography has five sizes and two label styles.** Settle one
    scale: `label` 11px/600/uppercase/0.6ls muted; `meta` 11px/400 muted;
    `body` 12px; `control` 13px; `display` 28px. Bump the 10px search-result
    chapter line to 11px. *Trivial.*

16. **Focus mode is just reading minus 74px of chrome.** Consider optically
    centring the column in the taller viewport, widening margins (not the
    measure), and hiding the cursor after 2s. *Moderate, band geometry.*

17. **In passing**: verify `image-rendering: pixelated` on a fractional HiDPI
    scale; the notice toast overlaps the status bar (offset it by 44px when a
    book is open); the toolbar theme button and the popover's theme chips can
    disagree (item 4 removes one); the settings heading says "Reading" but holds
    the theme — rename to "Display" or add a "Theme" label.

### Explicitly good — leave alone

The page itself (70ch measure, 1.6 leading, ragged right, centred column) —
"looks like a product, not a prototype". The Light and Sepia palettes. Search
result rows. TOC indentation and row rhythm. Escape backing out one layer at a
time. `PAGE_OVERLAP` on space/PgDn. The 280ms chapter slide+fade and the
Rust-driven eased scroll. Status bar proportions (the container is right, only
its contents are wrong). The token architecture — every colour fix above is a
one-line change *because* colours live in Rust and are pushed into Slint.

### Suggested order

1. Items 1, 3, 6 — small fixes, outsized return.
2. Item 2 — themed form controls.
3. Items 4, 5, 7, 8 — the chrome's information architecture.
4. Items 9, 13, 14 — discoverability and edge polish.
5. Items 10, 11, 12, 15, 16 — shelf, scrollbar, palette, type scale, focus.
