//! Flow invariants.
//!
//! A chapter becomes one continuous column, so the properties that matter are
//! ordering (blocks never overlap or swap), coverage (the column is as tall as
//! its content) and round-tripping (a scroll offset maps to a line and back).

mod support;

use folio_core::content::parse_chapter;
use folio_core::layout::{ChapterLayout, Flow, ImageInfo, Layouter, TypeSettings, flow};
use proptest::prelude::*;

const COLUMN: f32 = 600.0;
const VIEWPORT: f32 = 400.0;
const PAD: f32 = 28.0;

fn lay_out(xhtml: &str, font_px: f32) -> (Layouter, ChapterLayout, Flow) {
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
        VIEWPORT,
        "serif",
        "monospace",
        |_| None,
    );
    let flowed = flow(&layout, PAD);
    (layouter, layout, flowed)
}

#[test]
fn blocks_are_stacked_in_order_without_overlapping() {
    let (_, layout, flow) = lay_out(&support::paragraphs(40), 18.0);

    assert_eq!(
        flow.placements.len(),
        layout.blocks.iter().filter(|b| !b.lines.is_empty()).count(),
        "every non-empty block is placed exactly once"
    );

    let mut previous_bottom = 0.0_f32;
    let mut previous_index = None;
    for placement in &flow.placements {
        assert!(
            placement.y >= previous_bottom - 0.01,
            "block {} overlaps the one above it",
            placement.block
        );
        if let Some(previous) = previous_index {
            assert!(placement.block > previous, "blocks kept their order");
        }
        previous_bottom = placement.bottom();
        previous_index = Some(placement.block);
    }

    assert!(
        flow.height >= previous_bottom + PAD - 0.01,
        "the column is tall enough for its content plus the end padding"
    );
    assert!(flow.height > VIEWPORT, "fixture should be scrollable");
}

#[test]
fn only_the_blocks_near_the_viewport_are_visible() {
    let (_, _, flow) = lay_out(&support::paragraphs(40), 18.0);
    let top = flow.height / 2.0;
    let bottom = top + VIEWPORT;

    for placement in flow.visible(top, bottom) {
        assert!(
            placement.bottom() > top && placement.y < bottom,
            "visible() returned a block outside the band"
        );
    }
    let visible: Vec<_> = flow.visible(top, bottom).collect();
    assert!(!visible.is_empty(), "something is on screen mid-chapter");
    assert!(
        visible.len() < flow.placements.len(),
        "a band shows only part of a long chapter"
    );
}

#[test]
fn an_oversized_image_is_scaled_to_fit_the_window() {
    let mut layouter = Layouter::new();
    let chapter = parse_chapter(r#"<body><p>before</p><img src="huge.png"/><p>after</p></body>"#);
    let layout = layouter.layout_chapter(
        0,
        &chapter,
        &TypeSettings::default(),
        COLUMN,
        VIEWPORT,
        "serif",
        "monospace",
        |href| {
            Some(ImageInfo {
                path: href.to_string(),
                width: 1000,
                height: 8000, // far taller than any window
            })
        },
    );
    let image_block = layout
        .blocks
        .iter()
        .find(|block| block.image.is_some())
        .expect("image block");
    assert!(
        image_block.height <= VIEWPORT,
        "a tall figure is scaled so it can be seen in one screen"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// The line under the viewport's top edge must be the one whose box spans
    /// that offset: this is what makes a saved position mean anything.
    #[test]
    fn the_first_visible_line_spans_the_scroll_offset(
        paragraph_count in 5usize..30,
        fraction in 0.0f32..1.0,
    ) {
        let (_, layout, flow) = lay_out(&support::paragraphs(paragraph_count), 18.0);
        let scroll = flow.max_scroll(VIEWPORT) * fraction;
        let (block, line) = flow.first_line_at(&layout, scroll);

        let y = flow.line_y(&layout, block, line);
        let height = layout.blocks[block].lines[line].height;
        prop_assert!(
            y + height > scroll - 0.5,
            "line bottom {} is above the scroll offset {}", y + height, scroll
        );
        prop_assert!(
            y <= scroll + height + 0.5,
            "line top {} is far below the scroll offset {}", y, scroll
        );
    }

    /// Resolving a line back to an offset and asking again returns the same
    /// line, at every zoom step.
    #[test]
    fn lines_round_trip_through_offsets(
        paragraph_count in 5usize..20,
        font_px in prop::sample::select(folio_core::layout::ZOOM_STEPS.to_vec()),
    ) {
        let (_, layout, flow) = lay_out(&support::paragraphs(paragraph_count), font_px);
        for placement in flow.placements.iter() {
            let y = flow.line_y(&layout, placement.block, 0);
            let (block, line) = flow.first_line_at(&layout, y);
            prop_assert_eq!((block, line), (placement.block, 0));
        }
    }
}
