//! The medium contract.
//!
//! Every input medium (image, audio, document, website, mesh) is reduced to the
//! same thing before the engine sees it: an N x N grid of RGB cells. The engine
//! then permutes those cells. That is the *entire* interface between a medium and
//! the engine, which is what makes "LeBronify anything" one algorithm instead of
//! eleven.
//!
//! A [`Medium`] is produced by a reducer and consumed by the engine. Reducers are
//! trusted only for shape and bounds; [`Medium::validate`] re-checks those because
//! reducers read attacker-controlled files (a media parser returning a short
//! buffer must not turn into an out-of-bounds permutation later).

use serde::{Deserialize, Serialize};

/// A single source cell: one RGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pixel {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Pixel {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    #[inline(always)]
    pub fn tuple(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

/// An N x N grid of RGB cells: the engine's only input format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Medium {
    side: u32,
    cells: Vec<Pixel>,
}

impl Medium {
    /// Build a medium from a row-major grid. `cells.len()` must equal `side * side`.
    pub fn new(side: u32, cells: Vec<Pixel>) -> Result<Self, MediumError> {
        if side == 0 {
            return Err(MediumError::Empty);
        }
        if side > u16::MAX as u32 {
            // Cell coordinates are stored as u16 by the cost model; beyond this
            // the spatial term cannot represent a cell position.
            return Err(MediumError::TooLarge(side));
        }
        let expected = (side as usize) * (side as usize);
        if cells.len() != expected {
            return Err(MediumError::WrongCellCount {
                side,
                expected,
                actual: cells.len(),
            });
        }
        Ok(Self { side, cells })
    }

    /// Build from interleaved RGB bytes, row-major.
    pub fn from_rgb8(side: u32, bytes: &[u8]) -> Result<Self, MediumError> {
        let expected = (side as usize) * (side as usize) * 3;
        if bytes.len() != expected {
            return Err(MediumError::WrongByteCount {
                expected,
                actual: bytes.len(),
            });
        }
        let cells = bytes
            .chunks_exact(3)
            .map(|c| Pixel::rgb(c[0], c[1], c[2]))
            .collect();
        Self::new(side, cells)
    }

    #[inline]
    pub fn side(&self) -> u32 {
        self.side
    }

    #[inline]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    #[inline]
    pub fn cells(&self) -> &[Pixel] {
        &self.cells
    }

    #[inline]
    pub fn tuples(&self) -> Vec<(u8, u8, u8)> {
        self.cells.iter().map(Pixel::tuple).collect()
    }

    /// Interleaved RGB bytes, row-major.
    pub fn to_rgb8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.cells.len() * 3);
        for p in &self.cells {
            out.extend_from_slice(&[p.r, p.g, p.b]);
        }
        out
    }

    /// Cell at `(x, y)`, or `None` when out of bounds.
    pub fn get(&self, x: u32, y: u32) -> Option<Pixel> {
        if x >= self.side || y >= self.side {
            return None;
        }
        Some(self.cells[(y * self.side + x) as usize])
    }

    /// Re-check the invariants a reducer promised. Cheap, and called on every
    /// externally-sourced medium.
    pub fn validate(&self) -> Result<(), MediumError> {
        Medium::new(self.side, self.cells.clone()).map(|_| ())
    }
}
/// Why a reducer's output was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediumError {
    Empty,
    TooLarge(u32),
    WrongCellCount {
        side: u32,
        expected: usize,
        actual: usize,
    },
    WrongByteCount {
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for MediumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediumError::Empty => write!(f, "medium has zero cells"),
            MediumError::TooLarge(s) => {
                write!(f, "medium side {s} exceeds the u16 cell grid")
            }
            MediumError::WrongCellCount {
                side,
                expected,
                actual,
            } => write!(
                f,
                "medium side {side} needs {expected} cells, reducer produced {actual}"
            ),
            MediumError::WrongByteCount { expected, actual } => {
                write!(f, "expected {expected} RGB bytes, got {actual}")
            }
        }
    }
}

impl std::error::Error for MediumError {}
