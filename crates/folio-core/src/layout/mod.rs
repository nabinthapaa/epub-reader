//! Turning blocks into pixels: shaping, pagination and raster.

pub mod canvas;
pub mod engine;
pub mod fonts;
pub mod image;
pub mod paginate;
pub mod raster;
pub mod type_settings;

pub use canvas::Canvas;
pub use engine::{BlockLayout, ChapterLayout, ImageBox, ImageInfo, Layouter, LineBox, SpanRole};
pub use fonts::{FontStack, installed_families};
pub use image::DecodedImage;
pub use paginate::{Page, Pagination, Placement, paginate};
pub use raster::{LinkRegion, PageFrame, RenderedPage, render_page};
pub use type_settings::{Alignment, DEFAULT_ZOOM_INDEX, TypeSettings, ZOOM_STEPS};
