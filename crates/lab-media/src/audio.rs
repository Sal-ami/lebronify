//! Audio reduction: a short-time Fourier transform into an N x N picture.
//!
//! The audio tier's whole job is to turn a sound into the same N x N RGB grid
//! every other medium produces. The mapping chosen here is the standard one for
//! "show me a sound": time along the x axis, frequency up the y axis, magnitude
//! as color. That is a spectrogram, and it is a good fit for this program,
//! because the solver is looking for spatial structure it can move around - and
//! a spectrogram has plenty of it: harmonics are horizontal streaks, transients
//! are vertical ones.
//!
//! Two choices here are deliberate rather than incidental:
//!
//! * **Windows.** The signal is cut into windowed frames. A hard rectangular cut
//!   leaks energy across the whole spectrum, which turns a clean tone into a
//!   smear and makes the resulting grid noisy. A Hann window costs one multiply
//!   and removes most of that.
//! * **Log magnitude.** Linear magnitude puts nearly every bin near zero and
//!   spends the whole color range on the handful of loudest ones. `log1p` maps
//!   that onto something with visible structure, and matches how loudness is
//!   actually perceived.

use std::io::{Cursor, ErrorKind};

use lab_core::medium::{Medium, Pixel};
use lab_core::reducer::{ReduceError, Reducer, Tier};
use symphonia::core::audio::{AudioBuffer, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Length of the analysis window, in samples.
///
/// Fixed rather than derived from the clip length so that two different clips
/// reduced at the same side length are comparable, and so the result does not
/// depend on the sample rate.
pub const WINDOW: usize = 1024;

/// How far the window advances between columns, in samples.
///
/// A quarter window is the usual compromise: enough overlap that nothing is lost
/// between frames, without four times the transform cost.
pub const HOP: usize = WINDOW / 4;

/// Precomputed Hann window: `0.5 - 0.5 * cos(2*pi*n/(N-1))`.
pub fn hann_window(n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![1.0; n];
    }
    let denom = (n - 1) as f64;
    (0..n)
        .map(|i| {
            let t = 2.0 * std::f64::consts::PI * (i as f64) / denom;
            0.5 - 0.5 * t.cos()
        })
        .collect()
}

/// In-place iterative radix-2 Cooley-Tukey FFT.
///
/// `re` and `im` must be the same length and a power of two. The result is left
/// in the natural order. Written out rather than pulled in as a dependency
/// because the whole transform is forty lines and this keeps `lab-media`
/// dependency-light: the crate has no other numeric work.
pub fn fft_in_place(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert_eq!(n, im.len(), "fft: re and im must be the same length");
    debug_assert!(n.is_power_of_two(), "fft: length must be a power of two");
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies, doubling the transform length each pass.
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / (len as f64);
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0usize;
        while i < n {
            let (mut cur_r, mut cur_i) = (1.0f64, 0.0f64);
            for k in 0..(len / 2) {
                let (a, b) = (i + k, i + k + len / 2);
                let tr = cur_r * re[b] - cur_i * im[b];
                let ti = cur_r * im[b] + cur_i * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next_r = cur_r * wr - cur_i * wi;
                cur_i = cur_r * wi + cur_i * wr;
                cur_r = next_r;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// One spectrogram column per hop: the magnitude of the first `WINDOW/2 + 1`
/// bins, already log-scaled.
///
/// Indexed `columns[t][f]`, with `t` the frame index and `f` the frequency bin
/// starting at DC. The orientation choice is made once here rather than in each
/// renderer that consumes it.
pub fn log_magnitude_spectrogram(samples: &[f32]) -> Vec<Vec<f64>> {
    if samples.len() < WINDOW {
        return Vec::new();
    }
    let window = hann_window(WINDOW);
    let bins = WINDOW / 2 + 1;

    let mut out = Vec::with_capacity(samples.len() / HOP);
    let mut re = vec![0.0f64; WINDOW];
    let mut im = vec![0.0f64; WINDOW];

    let mut start = 0usize;
    while start + WINDOW <= samples.len() {
        for i in 0..WINDOW {
            re[i] = samples[start + i] as f64 * window[i];
            im[i] = 0.0;
        }
        fft_in_place(&mut re, &mut im);

        let mut col = Vec::with_capacity(bins);
        for k in 0..bins {
            let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
            // log1p rather than log: it is defined at zero, so silence maps to
            // 0.0 instead of -inf, and it needs no magic epsilon.
            col.push((1.0 + mag).ln());
        }
        out.push(col);
        start += HOP;
    }
    out
}

/// Scale a column-major spectrogram into 8-bit values.
///
/// Normalized by the loudest bin in the clip rather than by an absolute
/// reference, so a quiet recording is as legible as a loud one. A silent clip
/// has no maximum and produces all zeros instead of a division by zero.
pub fn normalize_to_bytes(columns: &[Vec<f64>]) -> Vec<Vec<u8>> {
    let max = columns
        .iter()
        .flat_map(|c| c.iter().copied())
        .fold(0.0f64, f64::max);

    if max <= 0.0 {
        return columns.iter().map(|c| vec![0u8; c.len()]).collect();
    }

    columns
        .iter()
        .map(|c| {
            c.iter()
                .map(|v| ((v / max).clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect()
        })
        .collect()
}

/// Resample `values` to exactly `side` positions by averaging the input range
/// each output position covers.
///
/// Averaging rather than nearest-neighbour because this runs as the last step of
/// a large reduction, and dropping bins would fold harmonics into noise. When
/// there are fewer inputs than outputs, each range is at least one and repeats.
pub fn resample_bytes(values: &[u8], side: usize) -> Vec<u8> {
    if side == 0 || values.is_empty() {
        return vec![0; side];
    }
    let n = values.len();
    (0..side)
        .map(|i| {
            let a = i * n / side;
            let mut b = (i + 1) * n / side;
            if b <= a {
                b = a + 1;
            }
            let slice = &values[a..b.min(n)];
            let sum: u32 = slice.iter().map(|v| *v as u32).sum();
            (sum / slice.len() as u32) as u8
        })
        .collect()
}

/// Map a magnitude to a color along a black -> orange -> white ramp.
///
/// Chosen because it is perceptually ordered and matches the target: quiet bins
/// are dark and loud ones arrive bright, so the solver has a reason to put the
/// loud cells where the target wants light.
pub fn magnitude_color(v: u8) -> (u8, u8, u8) {
    let t = v as f64 / 255.0;
    let stops = [
        (0.0f64, (0.0f64, 0.0f64, 0.0f64)),
        (0.35, (124.0, 18.0, 12.0)),
        (0.70, (247.0, 127.0, 0.0)),
        (1.0, (255.0, 255.0, 255.0)),
    ];
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let span = (t1 - t0).max(f64::EPSILON);
            let u = ((t - t0) / span).clamp(0.0, 1.0);
            let mix = |a: f64, b: f64| (a + (b - a) * u).round().clamp(0.0, 255.0) as u8;
            return (mix(c0.0, c1.0), mix(c0.1, c1.1), mix(c0.2, c1.2));
        }
    }
    (255, 255, 255)
}

/// Longest clip that analysis runs over, in seconds.
///
/// A spectrogram of an hour-long file at a fixed window is gigabytes of columns
/// for no extra detail, because the grid is at most 256 wide. Truncating is
/// therefore lossless as far as the output is concerned.
pub const MAX_ANALYSIS_SECONDS: f64 = 300.0;

/// Absolute ceiling on decoded frames, so a malformed header cannot ask for an
/// unbounded allocation.
pub const MAX_DECODED_FRAMES: u64 = 20_000_000;

/// Sample rate assumed when the container does not declare one.
const ASSUMED_RATE: u32 = 16_000;

/// RMS below which a clip counts as silent, in f32 sample units.
const SILENCE_RMS: f64 = 1e-4;

/// Decode any supported container to mono `f32` samples plus the sample rate.
///
/// Multichannel audio is averaged to mono rather than having a channel picked,
/// so a stereo mix and its mono fold-down produce the same picture.
///
/// Returns a human-readable reason on failure: a reducer's job is to explain
/// what was wrong with the input, and the caller turns that into `ReduceError`.
pub fn decode_to_mono_f32(input: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let cursor = Cursor::new(input.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    // No extension hint: the tier is chosen from the file name already, and the
    // probe sniffs the container, so a mislabelled file still decodes.
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("not a media container this build can read: {e}"))?;

    let mut format = probed.format;

    let track = format
        .default_track()
        .or_else(|| format.tracks().first())
        .ok_or_else(|| "the container has no audio track".to_string())?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let mut rate = params.sample_rate.unwrap_or(ASSUMED_RATE).max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| format!("no decoder for this codec: {e}"))?;

    // Built lazily from the first decoded frame: constructing it from the
    // container's declared parameters would mismatch (and panic on convert)
    // whenever the header understates the real format, which damaged files do.
    let mut converted: Option<AudioBuffer<f32>> = None;
    let mut mono: Vec<f32> = Vec::new();
    let mut max_frames = u64::MAX;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Hitting the end of the stream is the normal way out of this loop.
            Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("could not read the stream: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(ref decoded) => {
                let frames = decoded.frames();
                if frames == 0 {
                    continue;
                }

                if max_frames == u64::MAX {
                    max_frames =
                        ((rate as f64 * MAX_ANALYSIS_SECONDS) as u64).min(MAX_DECODED_FRAMES);
                }

                let stale = match &converted {
                    Some(buf) => frames > buf.capacity() || buf.spec() != decoded.spec(),
                    None => true,
                };
                if stale {
                    converted = Some(decoded.make_equivalent::<f32>());
                    rate = decoded.spec().rate.max(1);
                }
                let buf = converted.as_mut().expect("assigned immediately above");
                decoded.convert(buf);

                let channels = buf.spec().channels.count();
                if channels == 0 {
                    continue;
                }
                let inv = 1.0f32 / channels as f32;
                for i in 0..frames {
                    if mono.len() as u64 >= max_frames {
                        break;
                    }
                    let mut acc = 0.0f32;
                    for c in 0..channels {
                        acc += buf.chan(c)[i];
                    }
                    mono.push(acc * inv);
                }
                if mono.len() as u64 >= max_frames {
                    break;
                }
            }
            // One damaged frame mid-clip is not a reason to refuse the whole
            // clip: skip it and keep going, like a player would.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("decoding failed: {e}")),
        }
    }

    if mono.is_empty() {
        return Err("the clip decoded to no audio".to_string());
    }

    let sum_sq: f64 = mono.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    if (sum_sq / mono.len() as f64).sqrt() < SILENCE_RMS {
        return Err("the clip is silent".to_string());
    }

    Ok((mono, rate))
}

/// The audio tier: decode, fold to mono, STFT, color the magnitudes.
#[derive(Clone, Debug, Default)]
pub struct AudioReducer {
    pub crop: lab_core::CropScale,
}

impl AudioReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for AudioReducer {
    fn tier(&self) -> Tier {
        Tier::Audio
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Audio));
        }
        if side == 0 {
            return Err(ReduceError::Medium(lab_core::MediumError::Empty));
        }

        let (mono, _rate) = decode_to_mono_f32(input).map_err(ReduceError::Decode)?;

        let columns = log_magnitude_spectrogram(&mono);
        if columns.is_empty() {
            return Err(ReduceError::Empty(Tier::Audio));
        }
        let scaled = normalize_to_bytes(&columns);

        // The crop widget windows the timeline the same way it windows an image:
        // `scale` zooms in on the clip, `x` pans along it.
        let (start, end, _) = self.crop.columns(scaled.len());
        let window = &scaled[start..end];
        if window.is_empty() {
            return Err(ReduceError::Empty(Tier::Audio));
        }

        // One output row per grid row, low frequencies at the bottom of the
        // picture and therefore at the highest row index.
        let bins = window[0].len();
        let mut cells = Vec::with_capacity((side * side) as usize);
        for row in 0..side {
            let y0 = ((side - 1 - row) as usize) * bins / side as usize;
            let mut y1 = ((side - row) as usize) * bins / side as usize;
            if y1 <= y0 {
                y1 = y0 + 1;
            }
            let y1 = y1.min(bins);

            // Computed once per band and shared along the row, so the transform
            // runs once per column rather than once per cell.
            let per_column: Vec<Vec<u8>> = window
                .iter()
                .map(|col| resample_bytes(&col[y0..y1], side as usize))
                .collect();

            for x in 0..side as usize {
                let mut acc = 0u32;
                for col in &per_column {
                    acc += col[x] as u32;
                }
                let v = (acc / per_column.len() as u32) as u8;
                let (r, g, b) = magnitude_color(v);
                cells.push(Pixel::rgb(r, g, b));
            }
        }

        Medium::new(side, cells).map_err(ReduceError::from)
    }
}
