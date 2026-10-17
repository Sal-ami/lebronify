//! Document reduction: text becomes a coverage grid.
//!
//! One character is one pixel, and a pixel is inked when the character is not
//! whitespace. That keeps the thing the engine cares about, the shape of a page:
//! line lengths, paragraph gaps, indentation and centered headings all show up
//! as ink density. Markdown is not stripped first, because the markers are part
//! of the page too and removing them would flatten the layout.
//!
//! The characters themselves are kept alongside the ink, so [`crate::export`]
//! can hand back the real text with its cells moved rather than a grid of dots.

use image::{Rgb, RgbImage};
use lab_core::medium::Medium;
use lab_core::reducer::{ReduceError, Reducer, Tier};

use crate::finish_canvas;

const MAX_ROWS: usize = 512;
const MAX_COLS: usize = 512;

const INK: Rgb<u8> = Rgb([235, 235, 235]);
const PAPER: Rgb<u8> = Rgb([16, 16, 16]);

/// The document tier: text and Markdown to a page-shaped grid.
#[derive(Clone, Debug, Default)]
pub struct DocumentReducer {
    pub crop: lab_core::CropScale,
}

impl DocumentReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for DocumentReducer {
    fn tier(&self) -> Tier {
        Tier::Document
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Document));
        }
        let text = String::from_utf8_lossy(input);
        let rows = char_rows(&text);
        if !rows.iter().any(|r| r.iter().any(|c| !c.is_whitespace())) {
            return Err(ReduceError::Empty(Tier::Document));
        }
        let w = rows.iter().map(Vec::len).max().unwrap_or(1).max(1) as u32;
        let h = rows.len().max(1) as u32;
        let mut img = RgbImage::from_pixel(w, h, PAPER);
        for (y, row) in rows.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                if !c.is_whitespace() {
                    img.put_pixel(x as u32, y as u32, INK);
                }
            }
        }
        finish_canvas(Tier::Document, img, &self.crop, side)
    }
}

/// One row per line, capped at the canvas. The characters are kept rather than a
/// boolean, because the export moves the text itself.
pub(crate) fn char_rows(text: &str) -> Vec<Vec<char>> {
    let mut rows: Vec<Vec<char>> = Vec::new();
    for line in text.lines() {
        if rows.len() >= MAX_ROWS {
            break;
        }
        rows.push(line.chars().take(MAX_COLS).collect());
    }
    rows
}
