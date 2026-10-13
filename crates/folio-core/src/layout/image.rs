//! Image decoding for the reading pane.
//!
//! Raster formats go through `image`; SVG (which EPUB covers love) is rendered
//! once with resvg at a generous size and scaled down at draw time.

/// Decoded, straight-alpha RGBA pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Longest edge we rasterize an SVG at; it is scaled down to the column later.
const SVG_MAX_EDGE: f32 = 2000.0;

impl DecodedImage {
    /// Decode image bytes, sniffing SVG by content. Returns `None` for formats
    /// we cannot render, so the caller can fall back to alt text.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if looks_like_svg(bytes) {
            return decode_svg(bytes);
        }
        let decoded = image::load_from_memory(bytes).ok()?;
        let rgba = decoded.to_rgba8();
        Some(Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        })
    }

    /// Nearest-neighbour-free resize to exactly `width` x `height`.
    pub fn resized(&self, width: u32, height: u32) -> Option<Vec<u8>> {
        if width == 0 || height == 0 {
            return None;
        }
        if width == self.width && height == self.height {
            return Some(self.rgba.clone());
        }
        let source = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())?;
        let resized = image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::CatmullRom,
        );
        Some(resized.into_raw())
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head_len = bytes.len().min(512);
    let head = String::from_utf8_lossy(&bytes[..head_len]).to_ascii_lowercase();
    head.contains("<svg")
}

fn decode_svg(bytes: &[u8]) -> Option<DecodedImage> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if longest <= 0.0 {
        return None;
    }
    let scale = (SVG_MAX_EDGE / longest).clamp(0.01, 1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied alpha; the canvas works in straight alpha.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let a = p.alpha();
            let unpremultiply = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((c as u32 * 255) / a as u32).min(255) as u8
                }
            };
            [
                unpremultiply(p.red()),
                unpremultiply(p.green()),
                unpremultiply(p.blue()),
                a,
            ]
        })
        .collect();
    Some(DecodedImage {
        width,
        height,
        rgba,
    })
}
