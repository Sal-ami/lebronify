//! The reducer contract: how a medium becomes an N x N cell grid.
//!
//! A reducer is the *only* medium-specific code in the project. It takes raw
//! bytes and a side length and returns a [`Medium`]. It never sees the solver, the
//! target, or the cost model, so adding a medium cannot change engine behavior.
//!
//! [`TIERS`] is the single source of truth for which media exist and how far
//! along each one is. `cargo xtask matrix` reads it, and `docs/MEDIA-MATRIX.md` is
//! generated from that output, so the documentation cannot drift from the code.

use crate::medium::{Medium, MediumError};

/// One input medium family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    Image,
    Vector,
    Document,
    Website,
    Audio,
    Waveform,
    Video,
    Mesh,
    Table,
    Code,
    Palette,
}

impl Tier {
    pub const ALL: [Tier; 11] = [
        Tier::Image,
        Tier::Vector,
        Tier::Document,
        Tier::Website,
        Tier::Audio,
        Tier::Waveform,
        Tier::Video,
        Tier::Mesh,
        Tier::Table,
        Tier::Code,
        Tier::Palette,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tier::Image => "image",
            Tier::Vector => "vector",
            Tier::Document => "document",
            Tier::Website => "website",
            Tier::Audio => "audio",
            Tier::Waveform => "waveform",
            Tier::Video => "video",
            Tier::Mesh => "mesh",
            Tier::Table => "table",
            Tier::Code => "code",
            Tier::Palette => "palette",
        }
    }

    /// Two to four characters, for the dense UI's glyph labels.
    pub fn label(self) -> &'static str {
        match self {
            Tier::Image => "img",
            Tier::Vector => "vec",
            Tier::Document => "doc",
            Tier::Website => "web",
            Tier::Audio => "aud",
            Tier::Waveform => "wav",
            Tier::Video => "vid",
            Tier::Mesh => "msh",
            Tier::Table => "tbl",
            Tier::Code => "src",
            Tier::Palette => "pal",
        }
    }

    /// What actually happens to the input. Kept next to the code so the docs and
    /// the behavior are written in the same place.
    pub fn reduction(self) -> &'static str {
        match self {
            Tier::Image => "decode, crop to square, Lanczos3 downscale to NxN",
            Tier::Vector => "rasterize paths to a square, then downscale to NxN",
            Tier::Document => "one pixel per character, inked where it is not whitespace",
            Tier::Website => "parse pasted markup, draw a box/flow sketch into NxN",
            Tier::Audio => "STFT into an NxN spectrogram (time x frequency)",
            Tier::Waveform => "note events into an NxN piano roll, one row per pitch",
            Tier::Video => "sample frames across the clip into an NxN frame-mosaic",
            Tier::Mesh => "orthographic shaded projection with a z-buffer",
            Tier::Table => "map numeric columns to an NxN heatmap",
            Tier::Code => "tokenize, color by token class, fold lines into NxN",
            Tier::Palette => "sort swatches by hue/value into an NxN ramp",
        }
    }
}

/// How far along a tier is. Deliberately says "planned" rather than pretending,
/// so the matrix and the CLI both tell the truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// A reducer exists and the CLI can run it.
    Shipped,
    /// Design only; no reducer yet.
    Planned,
}

impl State {
    pub fn glyph(self) -> &'static str {
        match self {
            State::Shipped => "ok",
            State::Planned => "--",
        }
    }
}

/// Metadata for one tier: the row of the media matrix.
#[derive(Clone, Copy, Debug)]
pub struct TierSpec {
    pub tier: Tier,
    pub extensions: &'static [&'static str],
    pub state: State,
}

/// Every tier, in the order the matrix presents them. Extensions are lowercase
/// and do not include the dot.
pub const TIERS: &[TierSpec] = &[
    TierSpec {
        tier: Tier::Image,
        extensions: &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Vector,
        extensions: &["svg"],
        state: State::Planned,
    },
    TierSpec {
        tier: Tier::Document,
        extensions: &["txt", "md"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Website,
        extensions: &["html", "htm"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Audio,
        extensions: &["wav", "mp3", "ogg", "flac"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Waveform,
        extensions: &["mid", "midi"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Video,
        extensions: &["mp4", "webm", "mov"],
        state: State::Planned,
    },
    TierSpec {
        tier: Tier::Mesh,
        extensions: &["obj", "stl", "ply"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Table,
        extensions: &["csv", "tsv", "json"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Code,
        extensions: &["rs", "py", "js", "ts", "go", "c", "cpp", "h"],
        state: State::Shipped,
    },
    TierSpec {
        tier: Tier::Palette,
        extensions: &["hex", "gpl", "ase"],
        state: State::Shipped,
    },
];

/// A tier that is missing a `TierSpec` entry would silently vanish from the
/// matrix, so this is checked by `cargo xtask matrix`.
pub fn missing_specs() -> Vec<Tier> {
    Tier::ALL
        .iter()
        .copied()
        .filter(|t| !TIERS.iter().any(|s| s.tier == *t))
        .collect()
}

/// Pick a tier from a file extension, if any tier claims it.
pub fn tier_for_extension(ext: &str) -> Option<Tier> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    TIERS
        .iter()
        .find(|s| s.extensions.iter().any(|e| *e == ext))
        .map(|s| s.tier)
}

/// Reduce raw bytes of one medium into the engine's grid.
///
/// Implementations must be pure and total: same bytes plus same side must give
/// the same grid, and malformed input must be an error rather than a panic.
pub trait Reducer {
    fn tier(&self) -> Tier;

    /// Reduce `input` to a `side` x `side` grid.
    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError>;
}

/// Why a reduction failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReduceError {
    /// No tier claims this input.
    UnsupportedTier,
    /// The tier exists but its reducer is not written yet. Returning this
    /// instead of a fake grid keeps "planned" honest.
    NotImplemented(Tier),
    /// The decoder rejected the bytes.
    Decode(String),
    /// The media was decodable but has nothing to draw (zero-length, silent,
    /// no pages).
    Empty(Tier),
    /// The reduced grid was malformed. A bug in a reducer, not in the input.
    Medium(MediumError),
}

impl From<MediumError> for ReduceError {
    fn from(e: MediumError) -> Self {
        ReduceError::Medium(e)
    }
}

impl std::fmt::Display for ReduceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReduceError::UnsupportedTier => write!(f, "no tier handles this input"),
            ReduceError::NotImplemented(t) => {
                write!(f, "the {} tier is designed but not implemented yet", t.id())
            }
            ReduceError::Decode(e) => write!(f, "could not decode input: {e}"),
            ReduceError::Empty(t) => write!(f, "the {} input had nothing to draw", t.id()),
            ReduceError::Medium(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReduceError {}
