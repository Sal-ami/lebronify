//! Generation settings.
//!
//! Field semantics and defaults are copied from obamify's
//! `GenerationSettings`; the `*_RESOLUTION`/slider bounds are copied from the
//! reference GUI so a labronify run at the same settings is directly
//! comparable to an obamify run.

use serde::{Deserialize, Serialize};

/// The cell grid the proximity slider is calibrated against.
pub const CALIBRATION_SIDELEN: u32 = 128;
/// Default value of `proximity_importance` in the reference app.
pub const DEFAULT_PROXIMITY_IMPORTANCE: i64 = 13;
/// Reference GUI slider bounds: `resolution`.
pub const RESOLUTION_RANGE: (u32, u32) = (64, 256);
/// Reference GUI slider bounds: `proximity importance`.
pub const PROXIMITY_IMPORTANCE_RANGE: (i64, i64) = (0, 50);
/// Reference GUI slider bounds: crop `scale` (values below 1 are not allowed).
pub const CROP_SCALE_RANGE: (f32, f32) = (1.0, 5.0);
/// Reference GUI slider bounds: crop `x` / `y`.
pub const CROP_OFFSET_RANGE: (f32, f32) = (-1.0, 1.0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm {
    /// Hungarian / Kuhn-Munkres. Mathematically optimal, extremely slow above
    /// a few hundred cells.
    Optimal,
    /// Genetic hill-climb. The reference app's default.
    #[default]
    Genetic,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropScale {
    /// -1 all left, 0 center, 1 all right.
    pub x: f32,
    /// -1 all top, 0 center, 1 all bottom.
    pub y: f32,
    /// 1 fits within frame, >1 zooms in. Values below 1 are clamped to 1.
    pub scale: f32,
}

impl Default for CropScale {
    fn default() -> Self {
        Self::identity()
    }
}

impl CropScale {
    pub fn identity() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
        }
    }

    /// The square region this crop selects inside a `w` x `h` image, as
    /// `(x0, y0, side)`. Mirrors the reference `CropScale::apply` exactly.
    pub fn region(&self, w: u32, h: u32) -> (u32, u32, u32) {
        let s = self.scale.max(1.0);
        let base_side = w.min(h) as f32;
        let mut crop_side = (base_side / s).floor().max(1.0);
        crop_side = crop_side.min(w as f32).min(h as f32);

        let max_x_off = (w as f32 - crop_side).max(0.0);
        let max_y_off = (h as f32 - crop_side).max(0.0);

        let xn = (self.x.clamp(-1.0, 1.0) + 1.0) * 0.5;
        let yn = (self.y.clamp(-1.0, 1.0) + 1.0) * 0.5;

        let x0 = (xn * max_x_off).floor() as u32;
        let y0 = (yn * max_y_off).floor() as u32;
        (x0, y0, crop_side as u32)
    }

    /// The fractional input range this crop selects along a one-dimensional
    /// timeline of length `len`, as `(start, end, span)` where the first two are
    /// a half-open index range.
    ///
    /// This is the 1-D reading of the controls `region` applies to an image:
    /// `x` pans along the timeline and `scale` zooms in around the resulting
    /// window. A crop is a window onto a medium, so a non-image tier gets the
    /// same two knobs as the image tier rather than a second, inconsistent set.
    pub fn columns(&self, len: usize) -> (usize, usize, usize) {
        if len == 0 {
            return (0, 0, 0);
        }
        // A sub-range shorter than one sample would be empty, so it is floored
        // at one sample and capped at the input length, exactly as `region`
        // floors and caps `crop_side`.
        let s = f64::from(self.scale.max(1.0));
        let span = ((len as f64) / s).floor().max(1.0).min(len as f64) as usize;
        let max_off = len - span;
        let xn = f64::from(self.x.clamp(-1.0, 1.0) + 1.0) * 0.5;
        let start = ((xn * max_off as f64).floor() as usize).min(max_off);
        (start, start + span, span)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Cell grid side length. See `RESOLUTION_RANGE`.
    pub sidelen: u32,
    /// How hard the solver is pushed to keep cells near where they started.
    pub proximity_importance: i64,
    pub algorithm: Algorithm,
    pub source_crop: CropScale,
    pub target_crop: CropScale,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sidelen: CALIBRATION_SIDELEN,
            proximity_importance: DEFAULT_PROXIMITY_IMPORTANCE,
            algorithm: Algorithm::default(),
            source_crop: CropScale::identity(),
            target_crop: CropScale::identity(),
        }
    }
}

impl Settings {
    /// Adjust `proximity_importance` for the chosen resolution.
    ///
    /// The reference GUI does this at "start!" time so that the slider means the
    /// same thing at every resolution. Reproduced here because skipping it makes
    /// high-resolution runs behave differently from the reference.
    pub fn calibrated_proximity_importance(&self) -> i64 {
        (self.proximity_importance as f32 / (self.sidelen as f32 / CALIBRATION_SIDELEN as f32))
            as i64
    }

    /// Settings with the reference calibration already applied.
    pub fn calibrated(&self) -> Self {
        let mut s = self.clone();
        s.proximity_importance = self.calibrated_proximity_importance();
        s
    }
}
