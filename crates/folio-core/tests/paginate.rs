//! Pagination invariants.
//!
//! Pages are slices of the lines layout produced, so the properties that must
//! hold are partition-like: every line lands on exactly one page, in order, and
//! a position round-trips through a page and back at any zoom level.

mod support;

use folio_core::content::parse_chapter;
use folio_core::layout::paginate::{char_for_line, line_for_char, paginate};
use folio_core::layout::{ChapterLayout, Layouter, TypeSettings};
use proptest::prelude::*;

const COLUMN: f32 = 600.0;
const PAGE_HEIGHT: f32 = 400.0;

fn lay_out(xhtml: &str, font_px: f32) -> (Layouter, ChapterLayout) {
    let mut layouter = Layouter::new();
    let chapter = parse_chapter(xhtml);
    let settings = TypeSettings {
        font_px,
        ..TypeSettings::default()
    };
    let layout = layouter.layout_chapter(
        0,
        &chapter,
        &settings,
        COLUMN,
        PAGE_HEIGHT,
        "serif",
        "monospace",
        |_| None,
    );
    (layouter, layout)
}

/// Collect (block, line) pairs in page order.
fn placed_lines(pagination: &folio_core::layout::Pagination) -> Vec<(usize, usize)> {
    pagination
        .pages
        .iter()
        .flat_map(|page| {
            page.placements.iter().flat_map(|placement| {
                (placement.line_from..placement.line_to).map(move |line| (placement.block, line))
            })
        })
        .collect()
}

#[test]
fn every_line_is_placed_exactly_once_and_in_order() {
    let (_, layout) = lay_out(&support::paragraphs(40), 18.0);
    let pagination = paginate(&layout, PAGE_HEIGHT);

    let expected: Vec<(usize, usize)> = layout
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(index, block)| (0..block.lines.len()).map(move |line| (index, line)))
        .collect();

    assert_eq!(
        placed_lines(&pagination),
        expected,
        "pages must partition the chapter's lines, preserving order"
    );
    assert!(pagination.pages.len() > 1, "fixture should span pages");
}

#[test]
fn pages_do_not_overflow_unless_a_single_line_cannot_fit() {
    let (_, layout) = lay_out(&support::paragraphs(30), 18.0);
    let pagination = paginate(&layout, PAGE_HEIGHT);

    for page in &pagination.pages {
        let bottom = page
            .placements
            .iter()
            .map(|placement| {
                let block = &layout.blocks[placement.block];
                let height: f32 = block.lines[placement.line_from..placement.line_to]
                    .iter()
                    .map(|line| line.height)
                    .sum();
                placement.y + height
            })
            .fold(0.0_f32, f32::max);
        let single_line_page = page.placements.len() == 1
            && page.placements[0].line_to - page.placements[0].line_from == 1;
        assert!(
            bottom <= PAGE_HEIGHT + 0.5 || single_line_page,
            "page bottom {bottom} exceeds {PAGE_HEIGHT}"
        );
    }
}

#[test]
fn an_oversized_image_is_scaled_to_fit_its_own_page() {
    let mut layouter = Layouter::new();
    let chapter = parse_chapter(r#"<body><p>before</p><img src="huge.png"/><p>after</p></body>"#);
    let layout = layouter.layout_chapter(
        0,
        &chapter,
        &TypeSettings::default(),
        COLUMN,
        PAGE_HEIGHT,
        "serif",
        "monospace",
        |href| {
            Some(folio_core::layout::ImageInfo {
                path: href.to_string(),
                width: 1000,
                height: 8000, // far taller than any page
            })
        },
    );
    let image_block = layout
        .blocks
        .iter()
        .position(|block| block.image.is_some())
        .expect("image block");
    assert!(
        layout.blocks[image_block].height <= PAGE_HEIGHT,
        "a tall image must be scaled down to fit a page"
    );

    let pagination = paginate(&layout, PAGE_HEIGHT);
    let page_with_image = pagination
        .pages
        .iter()
        .position(|page| page.placements.iter().any(|p| p.block == image_block))
        .expect("image is placed");
    assert_eq!(
        pagination.pages[page_with_image].placements.len(),
        1,
        "a full-height image gets a page to itself"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// Recording a page's position and resolving it again must land on the same
    /// page, at every zoom step: this is what keeps the reader's place when the
    /// font size changes.
    #[test]
    fn position_round_trips_at_any_zoom(
        paragraph_count in 5usize..30,
        font_px in prop::sample::select(folio_core::layout::ZOOM_STEPS.to_vec()),
    ) {
        let (_, layout) = lay_out(&support::paragraphs(paragraph_count), font_px);
        let pagination = paginate(&layout, PAGE_HEIGHT);

        for (index, page) in pagination.pages.iter().enumerate() {
            let Some((block, line)) = page.first_line() else { continue };
            let char_offset = char_for_line(&layout, block, line);
            let resolved_line = line_for_char(&layout, block, char_offset);
            let resolved_page = pagination.page_of_line(block, resolved_line);
            prop_assert_eq!(
                resolved_page,
                index,
                "page {} round-tripped to {}", index, resolved_page
            );
        }
    }

    /// Whatever the text, pages stay a partition of the lines.
    #[test]
    fn lines_are_partitioned_for_arbitrary_text(
        words in prop::collection::vec("[a-z]{1,12}", 20..200),
    ) {
        let body = format!("<body><p>{}</p></body>", words.join(" "));
        let (_, layout) = lay_out(&body, 18.0);
        let pagination = paginate(&layout, PAGE_HEIGHT);

        let total_lines: usize = layout.blocks.iter().map(|b| b.lines.len()).sum();
        prop_assert_eq!(placed_lines(&pagination).len(), total_lines);
    }
}
