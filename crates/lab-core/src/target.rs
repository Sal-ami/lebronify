//! The LeBron target and its importance mask.
//!
//! These are the reference app's `target256.png` and `weights256.png`, renamed
//! and embedded. Two things must not change:
//!
//! * `weights256.png` is a *single-channel* mask stored as RGB. The weight is the
//!   red channel, 0..255. Reading it as luminance, averaging channels, or
//!   normalizing it to 0..1 silently changes every cost in the program.
//! * The images are resized with Lanczos3, matching `imageops::resize` in the
//!   reference. A different filter changes the silhouette's edge weights, and
//!   therefore which cells the solver considers important.

use image::imageops::FilterType;
use image::{ImageError, RgbImage};

use crate::medium::{Medium, MediumError};
use crate::settings::{CropScale, Settings};

/// Resolution of the embedded target and mask.
pub const TARGET_SIDELEN: u32 = 256;

/// `target256.png`: LeBron, 256x256, at the reference app's canonical framing.
pub const LEBRON_TARGET_PNG: &[u8] = include_bytes!("../assets/lebron-target-256.png");

/// `weights256.png`: per-cell importance mask, red channel only.
pub const LEBRON_WEIGHTS_PNG: &[u8] = include_bytes!("../assets/lebron-weights-256.png");

/// The target image at full resolution.
pub fn raw_target() -> Result<RgbImage, ImageError> {
    Ok(image::load_from_memory(LEBRON_TARGET_PNG)?.to_rgb8())
}

/// The importance mask at full resolution. Red channel only; see module docs.
pub fn raw_weights() -> Result<RgbImage, ImageError> {
    Ok(image::load_from_memory(LEBRON_WEIGHTS_PNG)?.to_rgb8())
}

/// Crop to a square with `crop`, then resize to `side` x `side` with Lanczos3.
pub fn crop_and_resize(img: &RgbImage, crop: &CropScale, side: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let (x0, y0, cs) = crop.region(w, h);
    let cropped = image::imageops::crop_imm(img, x0, y0, cs, cs).to_image();
    if cs == side {
        cropped
    } else {
        image::imageops::resize(&cropped, side, side, FilterType::Lanczos3)
    }
}

/// The LeBron target and its weight mask, cropped and resized per `settings`.
pub fn target_and_weights(settings: &Settings) -> Result<(Medium, Vec<i64>), TargetError> {
    let side = settings.sidelen;
    let img = crop_and_resize(&raw_target()?, &settings.target_crop, side);
    let target = Medium::from_rgb8(side, &img.into_raw())?;

    let weights_img = crop_and_resize(&raw_weights()?, &settings.target_crop, side);
    // Red channel, per-cell, 0..255.
    let weights = weights_img
        .pixels()
        .map(|p| p[0] as i64)
        .collect::<Vec<_>>();

    Ok((target, weights))
}

/// Reduce an arbitrary RGB image to the engine's medium format.
pub fn medium_from_image(
    img: &RgbImage,
    crop: &CropScale,
    side: u32,
) -> Result<Medium, MediumError> {
    let resized = crop_and_resize(img, crop, side);
    Medium::from_rgb8(side, &resized.into_raw())
}

/// Convenience: the target at a given side length with no cropping.
pub fn target_at(side: u32) -> Result<Medium, TargetError> {
    let s = Settings {
        sidelen: side,
        ..Default::default()
    };
    Ok(target_and_weights(&s)?.0)
}

#[derive(Clone, Debug)]
pub enum TargetError {
    /// The embedded PNG failed to decode. This can only happen if the asset was
    /// replaced with something broken.
    Image(String),
    Medium(MediumError),
}

impl From<ImageError> for TargetError {
    fn from(e: ImageError) -> Self {
        TargetError::Image(e.to_string())
    }
}

impl From<MediumError> for TargetError {
    fn from(e: MediumError) -> Self {
        TargetError::Medium(e)
    }
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetError::Image(e) => write!(f, "embedded LeBron asset failed to decode: {e}"),
            TargetError::Medium(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TargetError {}
