//! A plain RGBA surface the page is drawn into and the UI uploads as an image.

use crate::theme::Color;

/// Straight-alpha RGBA8 pixel buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Canvas {
    /// A canvas filled with `background`.
    pub fn new(width: u32, height: u32, background: Color) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            rgba.extend_from_slice(&[background.r, background.g, background.b, 255]);
        }
        Self {
            width,
            height,
            rgba,
        }
    }

    /// Bytes this canvas occupies, for the pixmap cache budget.
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    /// Alpha-blend one pixel. Out-of-bounds writes are ignored, which keeps
    /// glyph edges and image rounding from needing special cases.
    pub fn blend(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || color.a == 0 {
            return;
        }
        let index = ((y as u32 * self.width + x as u32) * 4) as usize;
        let alpha = color.a as u32;
        let inverse = 255 - alpha;
        for (offset, channel) in [color.r, color.g, color.b].into_iter().enumerate() {
            let dst = self.rgba[index + offset] as u32;
            self.rgba[index + offset] =
                (((channel as u32 * alpha) + (dst * inverse)) / 255).min(255) as u8;
        }
        self.rgba[index + 3] = 255;
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x + width).ceil() as i32;
        let y1 = (y + height).ceil() as i32;
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, color);
            }
        }
    }

    /// Draw a 1px dashed rectangle outline, used to frame missing images.
    pub fn stroke_dashed_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
        dash: i32,
    ) {
        let x0 = x.round() as i32;
        let y0 = y.round() as i32;
        let x1 = (x + width).round() as i32;
        let y1 = (y + height).round() as i32;
        let dash = dash.max(1);
        for px in x0..x1 {
            if (px - x0) % (dash * 2) < dash {
                self.blend(px, y0, color);
                self.blend(px, y1 - 1, color);
            }
        }
        for py in y0..y1 {
            if (py - y0) % (dash * 2) < dash {
                self.blend(x0, py, color);
                self.blend(x1 - 1, py, color);
            }
        }
    }

    /// Blit straight-alpha RGBA pixels at `(x, y)`.
    pub fn draw_rgba(&mut self, x: f32, y: f32, width: u32, height: u32, pixels: &[u8]) {
        if pixels.len() < (width * height * 4) as usize {
            return;
        }
        let x0 = x.round() as i32;
        let y0 = y.round() as i32;
        for row in 0..height {
            for column in 0..width {
                let index = ((row * width + column) * 4) as usize;
                let color = Color::rgba(
                    pixels[index],
                    pixels[index + 1],
                    pixels[index + 2],
                    pixels[index + 3],
                );
                self.blend(x0 + column as i32, y0 + row as i32, color);
            }
        }
    }
}
