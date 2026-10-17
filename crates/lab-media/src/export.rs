//! Native export: the medium's own format, with its cells moved.
//!
//! A render proves the assignment works, but it answers with a picture. A
//! document should answer with a document. This rebuilds the source in its native
//! format by moving whole cells, so the text, the numbers or the swatches end up
//! arranged the way the engine arranged their pixels.
//!
//! Only tiers whose reduction is a grid of real values can do this. Audio keeps a
//! magnitude spectrogram and the mesh keeps a shaded projection; neither holds
//! the samples or the vertices the source had, so there is nothing to move and
//! they return `None`. Images are their own export: the rendered PNG.

use lab_core::reducer::{ReduceError, Tier};
use lab_core::{Assignment, CropScale};

use crate::code;
use crate::document;
use crate::palette;
use crate::table;

/// The source in its own format with its cells moved, or `None` when the tier
/// has no inverse.
pub fn export_native(
    tier: Tier,
    input: &[u8],
    assignment: &Assignment,
    crop: CropScale,
) -> Result<Option<Vec<u8>>, ReduceError> {
    match tier {
        Tier::Document => {
            if input.is_empty() {
                return Err(ReduceError::Empty(Tier::Document));
            }
            let rows = document::char_rows(&String::from_utf8_lossy(input));
            if !rows.iter().any(|r| r.iter().any(|c| !c.is_whitespace())) {
                return Err(ReduceError::Empty(Tier::Document));
            }
            Ok(Some(lines_out(
                &moved_chars(rows, &crop, assignment),
                assignment.side(),
            )))
        }

        Tier::Code => {
            let rows = code::char_rows(input)?;
            Ok(Some(lines_out(
                &moved_chars(rows, &crop, assignment),
                assignment.side(),
            )))
        }

        Tier::Table => {
            let grid = table::parse_grid(input)?;
            let canvas = table::value_canvas(&grid);
            let moved = permute(
                &canvas,
                table::CANVAS,
                table::CANVAS,
                &crop,
                assignment,
                None,
            );
            Ok(Some(csv_out(&moved, assignment.side())))
        }

        Tier::Palette => {
            let swatches = palette::swatches(input)?;
            let (canvas, size) = palette::swatch_canvas(&swatches);
            let moved = permute(&canvas, size, size, &crop, assignment, None);
            Ok(Some(hex_out(&moved)))
        }

        _ => Ok(None),
    }
}

/// A character grid, letterboxed and moved.
fn moved_chars(rows: Vec<Vec<char>>, crop: &CropScale, assignment: &Assignment) -> Vec<char> {
    let w = rows.iter().map(Vec::len).max().unwrap_or(1).max(1) as u32;
    let h = rows.len().max(1) as u32;
    let mut canvas = vec![' '; (w * h) as usize];
    for (y, row) in rows.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            canvas[y * w as usize + x] = c;
        }
    }
    permute(&canvas, w, h, crop, assignment, ' ')
}

/// Sample one cell per position from a canvas, then move each cell where the
/// assignment sends it.
///
/// Geometry mirrors `finish_canvas` and `lab_core::target::crop_and_resize`:
/// letterbox the content into a square, take the crop region, and read the
/// centre of each cell's block. Moving a whole cell moves its block, so the
/// native file follows the picture.
fn permute<T: Copy>(
    canvas: &[T],
    w: u32,
    h: u32,
    crop: &CropScale,
    assignment: &Assignment,
    blank: T,
) -> Vec<T> {
    let side = assignment.side();
    let s = w.max(h);
    let (x0, y0, cs) = crop.region(s, s);
    let ox = ((s - w) / 2) as i64;
    let oy = ((s - h) / 2) as i64;
    let step = cs as f64 / f64::from(side);
    let mut out = vec![blank; (side * side) as usize];
    for (t, slot) in out.iter_mut().enumerate() {
        let src = assignment.at(t).unwrap_or(t) as u32;
        let cx = f64::from(x0) + (f64::from(src % side) + 0.5) * step;
        let cy = f64::from(y0) + (f64::from(src / side) + 0.5) * step;
        let px = cx as i64 - ox;
        let py = cy as i64 - oy;
        if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
            *slot = canvas[(py as u32 * w + px as u32) as usize];
        }
    }
    out
}

/// One row of `side` characters per line, as UTF-8.
fn lines_out(cells: &[char], side: u32) -> Vec<u8> {
    let side = side as usize;
    let mut out = String::with_capacity(cells.len() + side);
    for row in cells.chunks(side) {
        out.extend(row.iter());
        out.push('\n');
    }
    out.into_bytes()
}

/// One row of `side` comma-separated numbers per line; blanks stay blank.
fn csv_out(cells: &[Option<f64>], side: u32) -> Vec<u8> {
    let side = side as usize;
    let mut out = String::new();
    for row in cells.chunks(side) {
        let line = row
            .iter()
            .map(|v| v.map(|v| v.to_string()).unwrap_or_default())
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out.into_bytes()
}

/// One `#rrggbb` per swatch, in the moved order. Blank cells are skipped.
fn hex_out(cells: &[Option<[u8; 3]>]) -> Vec<u8> {
    let mut out = String::new();
    for c in cells.iter().flatten() {
        out.push_str(&format!("#{:02x}{:02x}{:02x}\n", c[0], c[1], c[2]));
    }
    out.into_bytes()
}
