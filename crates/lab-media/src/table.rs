//! Table reduction: numbers in a grid become a heat map.
//!
//! A CSV, TSV or JSON table is already a two-dimensional array of values, which
//! is the same shape the engine wants. The reduction normalizes each column's
//! numbers to 0..1 across the whole table and paints them with a heat ramp, so
//! the picture shows where the big and small values sit. Non-numeric cells and
//! blank lines stay at the background color rather than being read as zero,
//! which would otherwise flood the map with false lows.

use image::{Rgb, RgbImage};
use lab_core::medium::Medium;
use lab_core::reducer::{ReduceError, Reducer, Tier};

use crate::finish_canvas;

/// Largest table the reducer lays out. Past this the cells are smaller than one
/// pixel of the 512-wide canvas, so the extra rows only cost time.
const MAX_ROWS: usize = 512;
const MAX_COLS: usize = 512;

/// Side of the internal canvas, in pixels. The engine downscales it to NxN.
pub(crate) const CANVAS: u32 = 512;

/// The color of a cell with no number in it.
const BG: Rgb<u8> = Rgb([16, 16, 16]);

/// The table tier: parse a delimited file or JSON into a numeric grid, then
/// paint it.
#[derive(Clone, Debug, Default)]
pub struct TableReducer {
    pub crop: lab_core::CropScale,
}

impl TableReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for TableReducer {
    fn tier(&self) -> Tier {
        Tier::Table
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        let grid = parse_grid(input)?;
        let values: Vec<f64> = grid.iter().flatten().flatten().copied().collect();
        let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let span = if (hi - lo).abs() < 1e-12 {
            1.0
        } else {
            hi - lo
        };

        let canvas = value_canvas(&grid);
        let mut img = RgbImage::from_pixel(CANVAS, CANVAS, BG);
        for (i, v) in canvas.iter().enumerate() {
            let Some(v) = v else { continue };
            let x = (i as u32) % CANVAS;
            let y = (i as u32) / CANVAS;
            img.put_pixel(x, y, heat(((v - lo) / span).clamp(0.0, 1.0) as f32));
        }
        finish_canvas(Tier::Table, img, &self.crop, side)
    }
}

/// Parse the input into a numeric grid. Anything with no numbers at all is
/// [`ReduceError::Empty`], not a grid of zeroes.
pub(crate) fn parse_grid(input: &[u8]) -> Result<Vec<Vec<Option<f64>>>, ReduceError> {
    if input.is_empty() {
        return Err(ReduceError::Empty(Tier::Table));
    }
    let text = String::from_utf8_lossy(input);
    let grid = if starts_json(&text) {
        parse_json(&text)
    } else {
        parse_delimited(&text)
    };
    let rows = grid.len();
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    if rows == 0 || cols == 0 || grid.iter().flatten().flatten().next().is_none() {
        return Err(ReduceError::Empty(Tier::Table));
    }
    Ok(grid)
}

/// The raw values laid out on the canvas the engine downscales: one entry per
/// canvas pixel, `None` where no number sits.
pub(crate) fn value_canvas(grid: &[Vec<Option<f64>>]) -> Vec<Option<f64>> {
    let rows = grid.len();
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    let cw = (CANVAS as f64 / cols as f64).max(1.0);
    let ch = (CANVAS as f64 / rows as f64).max(1.0);
    let mut out = vec![None; (CANVAS * CANVAS) as usize];
    for (ry, row) in grid.iter().enumerate() {
        let y0 = (ry as f64 * ch) as u32;
        let y1 = (((ry + 1) as f64 * ch) as u32).clamp(y0 + 1, CANVAS);
        for (cx, cell) in row.iter().enumerate() {
            let Some(v) = cell else { continue };
            let x0 = (cx as f64 * cw) as u32;
            let x1 = (((cx + 1) as f64 * cw) as u32).clamp(x0 + 1, CANVAS);
            for y in y0..y1 {
                for x in x0..x1 {
                    out[(y * CANVAS + x) as usize] = Some(*v);
                }
            }
        }
    }
    out
}

/// The heat ramp: dark blue through teal and green to yellow.
fn heat(t: f32) -> Rgb<u8> {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.00, [68.0, 1.0, 84.0]),
        (0.25, [59.0, 82.0, 139.0]),
        (0.50, [33.0, 145.0, 140.0]),
        (0.75, [94.0, 201.0, 98.0]),
        (1.00, [253.0, 231.0, 37.0]),
    ];
    let t = t.clamp(0.0, 1.0);
    for w in STOPS.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let f = if (t1 - t0).abs() < 1e-6 {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            return Rgb([
                (c0[0] + (c1[0] - c0[0]) * f) as u8,
                (c0[1] + (c1[1] - c0[1]) * f) as u8,
                (c0[2] + (c1[2] - c0[2]) * f) as u8,
            ]);
        }
    }
    Rgb([253, 231, 37])
}

fn starts_json(text: &str) -> bool {
    matches!(text.trim_start().chars().next(), Some('{') | Some('['))
}

fn parse_delimited(text: &str) -> Vec<Vec<Option<f64>>> {
    let delim = pick_delimiter(text);
    let mut grid = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if grid.len() >= MAX_ROWS {
            break;
        }
        let row = line
            .split(delim)
            .take(MAX_COLS)
            .map(|f| f.trim().parse::<f64>().ok())
            .collect();
        grid.push(row);
    }
    grid
}

/// The delimiter the header agrees on: whichever of `,`, tab, `;` or `|` occurs
/// most in the first few lines. Defaults to a comma for a single-column file.
fn pick_delimiter(text: &str) -> char {
    let sample: String = text.lines().take(5).collect::<Vec<_>>().join("\n");
    [',', '\t', ';', '|']
        .into_iter()
        .max_by_key(|d| sample.matches(*d).count())
        .filter(|d| sample.contains(*d))
        .unwrap_or(',')
}

fn parse_json(text: &str) -> Vec<Vec<Option<f64>>> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => flatten(&value),
        Err(_) => Vec::new(),
    }
}

fn flatten(value: &serde_json::Value) -> Vec<Vec<Option<f64>>> {
    match value {
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                return Vec::new();
            }
            let all_scalar = items.iter().all(|i| {
                !matches!(
                    i,
                    serde_json::Value::Array(_) | serde_json::Value::Object(_)
                )
            });
            if all_scalar {
                vec![numbers(value)]
            } else {
                items.iter().map(numbers).collect()
            }
        }
        serde_json::Value::Object(_) => vec![numbers(value)],
        _ => Vec::new(),
    }
}

/// Every number reachable from `value`, in document order, with a `None`
/// placeholder for each leaf that is not a number. Arrays and objects are walked
/// depth first, so an object or an array of objects still lays out as a row
/// instead of reducing to nothing.
fn numbers(value: &serde_json::Value) -> Vec<Option<f64>> {
    match value {
        serde_json::Value::Array(items) => items.iter().flat_map(numbers).collect(),
        serde_json::Value::Object(map) => map.values().flat_map(numbers).collect(),
        other => vec![other.as_f64()],
    }
}
