//! Reducers.
//!
//! Each reducer turns some medium into the engine's N x N RGB grid and nothing
//! else. Reducers are the only place medium-specific knowledge is allowed to
//! live; the engine above them is deliberately medium-blind.
//!
//! Nine tiers are implemented: images, documents, web pages, audio, MIDI,
//! meshes, tables, source code and palettes. Vector art and video are still
//! declared in `lab_core::reducer::TIERS` and return
//! [`ReduceError::NotImplemented`] so the CLI and the matrix can tell the truth
//! about what exists.

pub mod audio;
pub mod code;
pub mod document;
pub mod export;
pub mod mesh;
pub mod palette;
pub mod table;
pub mod waveform;
pub mod website;

pub use export::export_native;

use image::{Rgb, RgbImage};
use lab_core::medium::Medium;
use lab_core::reducer::{ReduceError, Reducer, Tier};

/// Letterbox a reducer's picture into a square dark canvas and hand it to the
/// engine's crop + Lanczos3 path, so every tier reaches the cell grid the same
/// way the image tier does. Reducers build their picture at whatever size suits
/// the medium; this is the one place that size is normalized.
pub(crate) fn finish_canvas(
    tier: Tier,
    content: RgbImage,
    crop: &lab_core::CropScale,
    side: u32,
) -> Result<Medium, ReduceError> {
    if content.width() == 0 || content.height() == 0 || side == 0 {
        return Err(ReduceError::Empty(tier));
    }
    let s = content.width().max(content.height());
    let mut canvas = RgbImage::from_pixel(s, s, Rgb([16, 16, 16]));
    let ox = ((s - content.width()) / 2) as i64;
    let oy = ((s - content.height()) / 2) as i64;
    image::imageops::overlay(&mut canvas, &content, ox, oy);
    lab_core::target::medium_from_image(&canvas, crop, side).map_err(ReduceError::from)
}

/// The image tier: decode, crop to a square, Lanczos3 downscale.
#[derive(Clone, Debug, Default)]
pub struct ImageReducer {
    pub crop: lab_core::CropScale,
}

impl ImageReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for ImageReducer {
    fn tier(&self) -> Tier {
        Tier::Image
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Image));
        }

        // `image::load_from_memory` sniffs the container, so a mislabelled file
        // still decodes if its bytes are recognized, and a corrupt one is an
        // error instead of a panic. No `catch_unwind` here on purpose: every
        // decoder we enable returns a Result for bad input.
        let decoded = image::load_from_memory(input)
            .map_err(|e| ReduceError::Decode(e.to_string()))?
            .to_rgb8();

        if decoded.width() == 0 || decoded.height() == 0 {
            return Err(ReduceError::Empty(Tier::Image));
        }

        lab_core::target::medium_from_image(&decoded, &self.crop, side).map_err(ReduceError::from)
    }
}

/// Everything that is declared but not yet reduced.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlannedReducer {
    pub tier: Option<Tier>,
}

impl PlannedReducer {
    pub fn for_tier(tier: Tier) -> Self {
        Self { tier: Some(tier) }
    }
}

impl Reducer for PlannedReducer {
    fn tier(&self) -> Tier {
        self.tier.unwrap_or(Tier::Image)
    }

    fn reduce(&self, _input: &[u8], _side: u32) -> Result<Medium, ReduceError> {
        match self.tier {
            Some(t) => Err(ReduceError::NotImplemented(t)),
            None => Err(ReduceError::UnsupportedTier),
        }
    }
}

/// The reducer for a tier. Returns a [`PlannedReducer`] for tiers that are
/// declared but not implemented, which fails loudly rather than silently
/// substituting something else.
pub fn reducer_for(tier: Tier, crop: lab_core::CropScale) -> Box<dyn Reducer> {
    match tier {
        Tier::Image => Box::new(ImageReducer::new(crop)),
        Tier::Audio => Box::new(audio::AudioReducer::new(crop)),
        Tier::Website => Box::new(website::WebsiteReducer::new(crop)),
        Tier::Document => Box::new(document::DocumentReducer::new(crop)),
        Tier::Waveform => Box::new(waveform::WaveformReducer::new(crop)),
        Tier::Mesh => Box::new(mesh::MeshReducer::new(crop)),
        Tier::Table => Box::new(table::TableReducer::new(crop)),
        Tier::Code => Box::new(code::CodeReducer::new(crop)),
        Tier::Palette => Box::new(palette::PaletteReducer::new(crop)),
        other => Box::new(PlannedReducer::for_tier(other)),
    }
}

/// Reduce a file's bytes, choosing the tier from its extension.
pub fn reduce_file(
    bytes: &[u8],
    extension: &str,
    side: u32,
    crop: lab_core::CropScale,
) -> Result<Medium, ReduceError> {
    let tier =
        lab_core::reducer::tier_for_extension(extension).ok_or(ReduceError::UnsupportedTier)?;
    reducer_for(tier, crop).reduce(bytes, side)
}
