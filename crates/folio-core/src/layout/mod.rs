//! Turning blocks into pixels: shaping, flowing into a column, and raster.

pub mod canvas;
pub mod engine;
pub mod flow;
pub mod fonts;
pub mod image;
pub mod raster;
pub mod type_settings;

pub use canvas::Canvas;
pub use engine::{BlockLayout, ChapterLayout, ImageBox, ImageInfo, Layouter, LineBox, SpanRole};
pub use flow::{BlockPlacement, Flow, flow};
pub use fonts::{FontStack, installed_families};
pub use image::DecodedImage;
pub use raster::{BandFrame, LinkRegion, RenderedBand, render_band};
pub use type_settings::{Alignment, DEFAULT_ZOOM_INDEX, TypeSettings, ZOOM_STEPS};
