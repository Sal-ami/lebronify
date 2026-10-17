//! Renderers.
//!
//! Every format here is a *rendering of an assignment*. The assignment is the
//! artifact; a PNG, a GIF or a before/after strip is a view of it. That ordering
//! is why re-exporting from a saved assignment always reproduces the same image.
//!
//! This crate returns bytes rather than writing files, so it stays usable from
//! wasm32 and never touches the filesystem.

use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};
use lab_core::{Assignment, Medium};

/// Pixels per cell when rendering for display. A 128 cell grid at 4 px/cell is
/// a 512 px image: big enough to read, small enough to send through a canvas.
pub const DEFAULT_PIXEL_SCALE: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// Two media that had to line up did not.
    SideMismatch { expected: u32, actual: u32 },
    /// The encoder failed. Only reachable on an out-of-memory or an encoder bug.
    Encode(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::SideMismatch { expected, actual } => {
                write!(f, "grid side mismatch: expected {expected}, got {actual}")
            }
            ExportError::Encode(e) => write!(f, "could not encode image: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// A [`Medium`] as an image, one pixel per cell.
pub fn to_image(medium: &Medium) -> RgbImage {
    let side = medium.side();
    let mut img = RgbImage::new(side, side);
    for (i, p) in medium.cells().iter().enumerate() {
        let x = (i as u32) % side;
        let y = (i as u32) / side;
        img.put_pixel(x, y, Rgb([p.r, p.g, p.b]));
    }
    img
}

/// Nearest-neighbour upscale. Deliberately *not* a smooth filter: the cell grid is
/// the content, and blurring it would hide what the solver actually did.
pub fn upscale(img: &RgbImage, scale: u32) -> RgbImage {
    let scale = scale.max(1);
    if scale == 1 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let mut out = RgbImage::new(w * scale, h * scale);
    for y in 0..h {
        for x in 0..w {
            let p = *img.get_pixel(x, y);
            for dy in 0..scale {
                for dx in 0..scale {
                    out.put_pixel(x * scale + dx, y * scale + dy, p);
                }
            }
        }
    }
    out
}

/// Encode any image as PNG bytes.
pub fn encode_png(img: &RgbImage) -> Result<Vec<u8>, ExportError> {
    let mut cursor = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| ExportError::Encode(e.to_string()))?;
    Ok(cursor.into_inner())
}

/// Render a medium to a PNG at one pixel per cell.
pub fn medium_to_png(medium: &Medium) -> Result<Vec<u8>, ExportError> {
    encode_png(&to_image(medium))
}

/// Render a medium to a PNG at `scale` pixels per cell.
pub fn medium_to_png_scaled(medium: &Medium, scale: u32) -> Result<Vec<u8>, ExportError> {
    encode_png(&upscale(&to_image(medium), scale))
}

/// Render an assignment over its source: the "after" image.
///
/// Requires that the assignment and the source describe the same grid, because an
/// assignment for a different grid would index out of the source's cells.
pub fn render_png(
    source: &Medium,
    assignment: &Assignment,
    scale: u32,
) -> Result<Vec<u8>, ExportError> {
    if source.side() != assignment.side() {
        return Err(ExportError::SideMismatch {
            expected: assignment.side(),
            actual: source.side(),
        });
    }
    let rendered = lab_core::render(source, assignment);
    medium_to_png_scaled(&rendered, scale)
}
/// A before/after strip: source on the left, result on the right, separated by a
/// one-pixel rule. This is the generic form of the before/after view; the audio
/// tier renders its waveform pair through the same function.
pub fn before_after_strip(
    before: &Medium,
    after: &Medium,
    scale: u32,
    rule_rgb: [u8; 3],
) -> Result<RgbImage, ExportError> {
    if before.side() != after.side() {
        return Err(ExportError::SideMismatch {
            expected: before.side(),
            actual: after.side(),
        });
    }
    let scale = scale.max(1);
    let side = before.side() * scale;
    let mut out = RgbImage::new(side * 2 + 1, side);

    let left = upscale(&to_image(before), scale);
    let right = upscale(&to_image(after), scale);

    for y in 0..side {
        for x in 0..side {
            out.put_pixel(x, y, *left.get_pixel(x, y));
            out.put_pixel(x + side + 1, y, *right.get_pixel(x, y));
        }
        out.put_pixel(side, y, Rgb(rule_rgb));
    }
    Ok(out)
}

/// The before/after strip as PNG bytes.
pub fn before_after_png(
    before: &Medium,
    after: &Medium,
    scale: u32,
    rule_rgb: [u8; 3],
) -> Result<Vec<u8>, ExportError> {
    encode_png(&before_after_strip(before, after, scale, rule_rgb)?)
}
