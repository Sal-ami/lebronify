//! MIDI reduction: note events become a piano roll.
//!
//! A piano roll is the natural picture of a MIDI file: time runs across, pitch
//! up the other axis, and a note is a bar whose brightness follows its velocity.
//! The parser is written out rather than pulled in as a dependency because MIDI
//! is a small, well-specified format and the tile only needs note on and note
//! off events.

use image::{Rgb, RgbImage};
use lab_core::medium::Medium;
use lab_core::reducer::{ReduceError, Reducer, Tier};

use crate::finish_canvas;

/// Notes any one file may contribute, so a malformed or gigantic file cannot
/// exhaust memory. Past this the roll is truncated.
const MAX_NOTES: usize = 200_000;

/// Piano roll size before it is downscaled to the cell grid.
const CANVAS_W: u32 = 512;
const PITCHES: u32 = 128;

/// One note, in ticks from the start of its track.
struct Note {
    pitch: u8,
    start: u64,
    end: u64,
    vel: u8,
}

/// The MIDI tier: parse note events, paint a piano roll.
#[derive(Clone, Debug, Default)]
pub struct WaveformReducer {
    pub crop: lab_core::CropScale,
}

impl WaveformReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for WaveformReducer {
    fn tier(&self) -> Tier {
        Tier::Waveform
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Waveform));
        }
        let notes = parse(input).map_err(ReduceError::Decode)?;
        if notes.is_empty() {
            return Err(ReduceError::Empty(Tier::Waveform));
        }
        let last = notes
            .iter()
            .map(|n| n.end.max(n.start))
            .max()
            .unwrap_or(1)
            .max(1);

        let mut img = RgbImage::from_pixel(CANVAS_W, PITCHES, Rgb([8, 8, 16]));
        for n in &notes {
            let x_of = |t: u64| (t as f64 / last as f64 * (CANVAS_W - 1) as f64) as u32;
            let x0 = x_of(n.start).min(CANVAS_W - 1);
            let x1 = x_of(n.end.max(n.start + 1)).min(CANVAS_W - 1).max(x0);
            let y = PITCHES - 1 - n.pitch.min(127) as u32;
            let b = 60 + (n.vel as u32) * 195 / 127;
            let bright = Rgb([b as u8, (b * 3 / 4) as u8, 60]);
            for x in x0..=x1 {
                let px = img.get_pixel_mut(x, y);
                *px = Rgb([
                    px.0[0].max(bright.0[0]),
                    px.0[1].max(bright.0[1]),
                    px.0[2].max(bright.0[2]),
                ]);
            }
        }
        finish_canvas(Tier::Waveform, img, &self.crop, side)
    }
}

fn parse(bytes: &[u8]) -> Result<Vec<Note>, String> {
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
        return Err("not a MIDI file: the MThd header is missing".into());
    }
    let mut p = 8 + be_u32(bytes, 4) as usize;
    if p > bytes.len() {
        return Err("MIDI header length runs past the end of the file".into());
    }
    let mut open: Vec<(u8, u64, u8)> = Vec::new();
    let mut notes = Vec::new();
    while p + 8 <= bytes.len() {
        let id = &bytes[p..p + 4];
        let len = be_u32(bytes, p + 4) as usize;
        let start = p + 8;
        let end = start.saturating_add(len).min(bytes.len());
        if id == b"MTrk" {
            parse_track(&bytes[start..end], &mut open, &mut notes);
        }
        p = start + len;
    }
    for (pitch, start, vel) in open {
        notes.push(Note {
            pitch,
            start,
            end: start,
            vel,
        });
    }
    Ok(notes)
}

fn parse_track(b: &[u8], open: &mut Vec<(u8, u64, u8)>, notes: &mut Vec<Note>) {
    let mut p = 0usize;
    let mut t = 0u64;
    let mut running = 0u8;
    while p < b.len() && notes.len() < MAX_NOTES {
        t += varint(b, &mut p);
        if p >= b.len() {
            break;
        }
        let mut status = b[p];
        if status & 0x80 != 0 {
            p += 1;
            if status < 0xF0 {
                running = status;
            }
        } else {
            status = running;
            if running == 0 {
                break;
            }
        }
        match status & 0xF0 {
            0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => {
                if p + 2 > b.len() {
                    break;
                }
                let (d1, d2) = (b[p], b[p + 1]);
                p += 2;
                let kind = status & 0xF0;
                if kind == 0x90 && d2 > 0 {
                    open.push((d1, t, d2));
                } else if kind == 0x80 || kind == 0x90 {
                    if let Some(i) = open.iter().rposition(|&(pitch, _, _)| pitch == d1) {
                        let (pitch, start, vel) = open.remove(i);
                        notes.push(Note {
                            pitch,
                            start,
                            end: t,
                            vel,
                        });
                    }
                }
            }
            0xC0 | 0xD0 => {
                if p + 1 > b.len() {
                    break;
                }
                p += 1;
            }
            _ => {
                if status == 0xFF {
                    if p >= b.len() {
                        break;
                    }
                    p += 1;
                    let len = varint(b, &mut p) as usize;
                    p = p.saturating_add(len).min(b.len());
                } else if status == 0xF0 || status == 0xF7 {
                    let len = varint(b, &mut p) as usize;
                    p = p.saturating_add(len).min(b.len());
                } else {
                    break;
                }
            }
        }
    }
}

fn varint(b: &[u8], p: &mut usize) -> u64 {
    let mut v = 0u64;
    for _ in 0..4 {
        if *p >= b.len() {
            break;
        }
        let byte = b[*p];
        *p += 1;
        v = (v << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            break;
        }
    }
    v
}

fn be_u32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
