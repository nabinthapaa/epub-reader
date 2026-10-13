# Overnight plan — polish Folio

Written 2026-10-13 23:0x local, for execution after the usage limit resets
(~01:02 local, 2026-10-14). The user is asleep and wants a polished product by
morning, then a code review at the very end.

**Read this file in full before doing anything.**

## Hard constraints

1. **Do not open GUI windows.** The user is asleep; `./target/release/folio`
   would pop a window onto their desktop and steal focus. All visual checks go
   through the headless renderer:
   `cargo run -p folio --example ui_shot -- <scene> <out.png> 1100x820 [BOOK]`
   with scenes `shelf · reading · toc · search · settings · focus`, and
   `FOLIO_SHOT_THEME=light|sepia|dark` to pick a palette.
   Books for testing: `~/.cache/folio-testbooks/{11,84,2701}.epub` (Alice,
   Frankenstein, Moby Dick).
2. **Stay inside `~/projects/folio`.** No `sudo` (it is blocked and counts
   against `pam_faillock`), no system packages, no pushing anywhere, no
   touching the user's other files. `~/.local/bin/folio` and the desktop entry
   are the one exception: refresh the installed binary at the very end.
3. **Never use `pkill -f` with a pattern that matches the running shell.** It
   kills the shell itself (learned the hard way). Use `pgrep -x folio`.
4. **Everything must stay green**: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`. Run all three before every commit.
5. **Commit each round** with a real message explaining what changed and why.
6. If something is ambiguous or risky, **leave it undone and write it in the
   report** rather than guessing. The user prefers an honest gap to a wrong fix.

## What Folio is (short version)

A desktop EPUB reader: Rust core (`crates/folio-core`, no Slint dependency) +
Slint UI (`crates/folio`). Chapter text is shaped with cosmic-text and
rasterized into an RGBA "band" ~2.2 screens tall that Slint shows as one Image
and slides by a scroll offset. Reading inside a chapter is continuous scrolling;
crossing chapters is an animated slide+fade. Publisher CSS is discarded on
purpose; Folio imposes its own typography. See `README.md` for the architecture
and the deliberate non-goals.

**Do not implement these** (settled design decisions): text selection/copy,
hyphenation, honoring publisher CSS, a paginated mode, folder-scanning library.

## The loop

Repeat up to **5 rounds**, stopping early when a review round turns up nothing
worth doing, and stopping unconditionally by **06:30 local**:

1. Read `UI-REVIEW.md` (the latest review findings, newest section at the top).
2. Implement the highest-value items that are (a) within the constraints above
   and (b) not on the non-goals list. Prefer changes that are cheap in Slint
   markup over changes to the Rust rasterizer, unless a rasterizer change is
   clearly the right fix.
3. Re-render every scene with `ui_shot` into `.overnight/shots-round-N/`.
4. Run fmt + clippy + tests. Fix anything broken before continuing.
5. Commit.
6. Spawn **one** read-only UI review agent (`subagent_type: "Plan"`, which has
   no Edit/Write tools) pointed at the new screenshots, telling it the same
   constraints and non-goals, and asking for a prioritized, concrete critique.
   Prepend its findings to `UI-REVIEW.md` under a dated heading.
7. Append one short paragraph to `.overnight/state.md`: round number, what
   changed, what is still open. This is the resume point if the session dies.

## After the loop

1. Run a **code review** over everything since commit `1ae6535`:
   `/code-review high` if the skill is available, otherwise spawn one
   general-purpose agent to review `git diff 1ae6535..HEAD` for correctness
   bugs, leaks, panics, and Slint/Rust misuse.
2. Fix what is genuinely wrong. Push back (in writing, in the report) on
   anything you judge to be a false positive — do not change code just to
   silence a reviewer.
3. Re-run fmt + clippy + tests, commit.
4. Refresh the installed binary:
   `cargo build --release && install -Dm755 target/release/folio ~/.local/bin/folio && strip ~/.local/bin/folio`
5. Write `OVERNIGHT-REPORT.md` at the repo root:
   - what changed, round by round, with commit hashes
   - before/after screenshot paths
   - what you deliberately did not do, and why
   - anything that needs the user's decision
   - the honest state of the test suite and lints
6. Touch `.overnight/DONE`.

## Current state (2026-10-13 23:0x)

- Commits: `1ae6535` (shelf return + headless shot tool), `cdfe836` (shelf and
  toolbar layout fixes), `b1df69e` (xdg app id), `602391e` (scrolling rework),
  `4e29114` (settings panel), `3dbf6ce` (initial).
- Tests: 33 passing (8 core unit, 5 flow, 8 parse, 11 reading, 1 UI scenario
  bundle). Clippy clean at `-D warnings`. fmt clean.
- Installed at `~/.local/bin/folio`, desktop entry + `.epub` association live.
- A first UI review was in flight when this was written; its findings are in
  `UI-REVIEW.md` if it completed, and that file is the starting point.
