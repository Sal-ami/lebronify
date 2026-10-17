//! labronify's engine.
//!
//! One algorithm, any medium. Every input is reduced to an N x N grid of RGB
//! cells, the engine finds the permutation of those cells that best matches the
//! LeBron target, and the result is an assignment array that every output format
//! is merely a rendering of.
//!
//! The cost model, the solvers' structure, the target assets and the defaults are
//! ported from obamify (MIT, (c) 2025 Spu7Nix) and are intended to behave
//! identically. See `NOTICE` and `docs/DECISIONS.md`. This crate does no IO
//! beyond decoding its own embedded assets, so the same code runs natively and
//! under wasm32.

pub mod cost;
pub mod medium;
pub mod reducer;
pub mod solvers;
pub mod target;

mod settings;

pub use medium::{Medium, MediumError, Pixel};
pub use settings::{
    Algorithm, CropScale, Settings, CALIBRATION_SIDELEN, CROP_OFFSET_RANGE, CROP_SCALE_RANGE,
    DEFAULT_PROXIMITY_IMPORTANCE, PROXIMITY_IMPORTANCE_RANGE, RESOLUTION_RANGE,
};
pub use solvers::{
    solve, solve_from, solve_traced, GenSink, Solution, SolveOutcome, CONVERGED_MAX_DIST,
    CONVERGED_SWAPS, GENETIC_SEED, MAX_DIST_DECAY, MAX_DIST_FLOOR, SWAPS_PER_GENERATION_PER_PIXEL,
};
pub use target::{target_and_weights, TargetError};

use serde::{Deserialize, Serialize};

use cost::cost;

/// A proven permutation of source cells into target positions.
///
/// The constructor is the only way to build one, and it rejects anything that is
/// not a permutation. That matters because the artifact is the thing that gets
/// saved, shared and reloaded: a malformed assignment must fail loudly at the
/// boundary rather than produce a scrambled render later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    side: u32,
    cells: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignmentError {
    WrongLength {
        side: u32,
        expected: usize,
        actual: usize,
    },
    /// The same source cell was used twice.
    DuplicateSource { index: usize, source: usize },
    /// A source index was outside `0..side*side`.
    OutOfRange {
        index: usize,
        source: usize,
        side: u32,
    },
}

impl std::fmt::Display for AssignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssignmentError::WrongLength {
                side,
                expected,
                actual,
            } => write!(
                f,
                "assignment for a {side}x{side} grid needs {expected} entries, got {actual}"
            ),
            AssignmentError::DuplicateSource { index, source } => write!(
                f,
                "entry {index} reuses source cell {source}; an assignment must be a permutation"
            ),
            AssignmentError::OutOfRange {
                index,
                source,
                side,
            } => write!(
                f,
                "entry {index} has source cell {source}, outside 0..{} for side {side}",
                (*side as usize) * (*side as usize)
            ),
        }
    }
}

impl std::error::Error for AssignmentError {}

impl Assignment {
    pub fn new(side: u32, cells: Vec<usize>) -> Result<Self, AssignmentError> {
        let n = (side as usize) * (side as usize);
        if cells.len() != n {
            return Err(AssignmentError::WrongLength {
                side,
                expected: n,
                actual: cells.len(),
            });
        }
        let mut seen = vec![false; n];
        for (index, &source) in cells.iter().enumerate() {
            if source >= n {
                return Err(AssignmentError::OutOfRange {
                    index,
                    source,
                    side,
                });
            }
            if seen[source] {
                return Err(AssignmentError::DuplicateSource { index, source });
            }
            seen[source] = true;
        }
        Ok(Self { side, cells })
    }

    #[inline]
    pub fn side(&self) -> u32 {
        self.side
    }

    #[inline]
    pub fn as_slice(&self) -> &[usize] {
        &self.cells
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Source cell index placed at target position `i`.
    #[inline]
    pub fn at(&self, i: usize) -> Option<usize> {
        self.cells.get(i).copied()
    }

    /// The inverse permutation: where source cell `i` ended up.
    pub fn inverse(&self) -> Assignment {
        let mut out = vec![0usize; self.cells.len()];
        for (target, &source) in self.cells.iter().enumerate() {
            out[source] = target;
        }
        Assignment::new(self.side, out).expect("inverse of a permutation is a permutation")
    }
}

/// Total cost of an assignment under the frozen cost model.
///
/// This is the scoring function for the whole project. Renders are not the
/// source of truth; this is.
pub fn evaluate(
    source: &Medium,
    target: &Medium,
    assignment: &[usize],
    weights: &[i64],
    proximity_importance: i64,
) -> Result<i64, AssignmentError> {
    let side = source.side();
    let assignment = Assignment::new(side, assignment.to_vec())?;
    Ok(evaluate_checked(
        source,
        target,
        &assignment,
        weights,
        proximity_importance,
    ))
}

/// Same as [`evaluate`] for an already-validated assignment.
pub fn evaluate_checked(
    source: &Medium,
    target: &Medium,
    assignment: &Assignment,
    weights: &[i64],
    proximity_importance: i64,
) -> i64 {
    let side = source.side() as usize;
    let source_t = source.tuples();
    let target_t = target.tuples();
    let mut total = 0i64;
    for (target_idx, &source_idx) in assignment.as_slice().iter().enumerate() {
        total += cost(
            ((source_idx % side) as u16, (source_idx / side) as u16),
            ((target_idx % side) as u16, (target_idx / side) as u16),
            source_t[source_idx],
            target_t[target_idx],
            weights[target_idx],
            proximity_importance,
        );
    }
    total
}

/// Render an assignment over a source medium: the "after" picture.
pub fn render(source: &Medium, assignment: &Assignment) -> Medium {
    let source_t = source.tuples();
    let cells = assignment
        .as_slice()
        .iter()
        .map(|&s| {
            let (r, g, b) = source_t[s];
            Pixel::rgb(r, g, b)
        })
        .collect();
    Medium::new(assignment.side(), cells).expect("render preserves the side length")
}

/// The saved artifact: the assignment plus enough context to re-render it.
///
/// `assignments.json` on its own is a permutation with no way to know which
/// framing produced it, so the settings are stored alongside. The bare array is
/// kept readable so a reference-app `assignments.json` can still be loaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    /// Format marker. Bumped only if the meaning of the fields changes.
    pub format: String,
    /// Side length of the cell grid.
    pub sidelen: u32,
    /// Settings in effect when this was solved, after calibration.
    pub settings: Settings,
    /// `assignment[target_index] = source_index`.
    pub assignment: Vec<usize>,
}

pub const ARTIFACT_FORMAT: &str = "labronify.assignment/1";

impl Artifact {
    pub fn new(settings: Settings, assignment: &Assignment) -> Self {
        Self {
            format: ARTIFACT_FORMAT.to_string(),
            sidelen: assignment.side(),
            settings,
            assignment: assignment.as_slice().to_vec(),
        }
    }

    /// Validate the stored permutation.
    pub fn to_assignment(&self) -> Result<Assignment, AssignmentError> {
        Assignment::new(self.sidelen, self.assignment.clone())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(s: &str) -> Result<Self, ArtifactParseError> {
        let artifact: Artifact =
            serde_json::from_str(s).map_err(|e| ArtifactParseError::Json(e.to_string()))?;
        if artifact.format != ARTIFACT_FORMAT {
            return Err(ArtifactParseError::UnknownFormat(artifact.format));
        }
        Ok(artifact)
    }

    /// Read a bare permutation array, as written by the reference app's
    /// `assignments.json`. The caller must supply the side length because the
    /// bare format does not carry one.
    pub fn from_bare_array(side: u32, s: &str) -> Result<Self, ArtifactParseError> {
        let cells: Vec<usize> =
            serde_json::from_str(s).map_err(|e| ArtifactParseError::Json(e.to_string()))?;
        let assignment = Assignment::new(side, cells)?;
        Ok(Self {
            format: ARTIFACT_FORMAT.to_string(),
            sidelen: side,
            settings: Settings {
                sidelen: side,
                ..Default::default()
            },
            assignment: assignment.as_slice().to_vec(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactParseError {
    Json(String),
    UnknownFormat(String),
    Assignment(AssignmentError),
}

impl From<AssignmentError> for ArtifactParseError {
    fn from(e: AssignmentError) -> Self {
        ArtifactParseError::Assignment(e)
    }
}

impl std::fmt::Display for ArtifactParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactParseError::Json(e) => write!(f, "artifact is not valid JSON: {e}"),
            ArtifactParseError::UnknownFormat(s) => write!(
                f,
                "unknown artifact format {s:?}, expected {ARTIFACT_FORMAT:?}"
            ),
            ArtifactParseError::Assignment(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ArtifactParseError {}

impl From<solvers::Solution> for Assignment {
    fn from(s: solvers::Solution) -> Self {
        let side = (s.assignment.len() as f64).sqrt() as u32;
        Assignment::new(side, s.assignment).expect("solvers return permutations")
    }
}
