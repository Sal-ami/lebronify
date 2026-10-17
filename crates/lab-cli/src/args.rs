//! Argument parsing.
//!
//! Hand-rolled on purpose: the command surface is small and fixed, and keeping it
//! dependency-free means `labronify --help` costs nothing to run. Every error
//! message names the flag that was wrong, because a CLI that says "invalid
//! arguments" wastes the user's time.

use std::path::PathBuf;

use lab_core::{Algorithm, CropScale};

pub const USAGE: &str = "\
labronify - turn any medium into LeBron. Nothing leaves this machine.

usage:
  labronify <command> [options]

commands:
  tiers                       list every medium tier and its state
  reduce  --in F --out F      reduce a medium to the N x N grid the engine sees
  solve   --in F --out F      find the assignment that turns --in into LeBron
  render  --in F --out F      render an assignment back to pixels
  export  --in F --out F      write the medium in its own format, cells moved.
                              needs --assignment. only for document, code, table
                              and palette; the others have no inverse

options:
  --in PATH                   input medium (extension picks the tier)
  --out PATH                  output file (.png for render/reduce, .json for solve)
  --side N                    cell grid side length, default 128
  --crop X,Y,SCALE            source crop: -1..1, -1..1, 1..5. default 0,0,1
  --target-crop X,Y,SCALE     same, for the LeBron target. default 0,0,1
  --proximity N               0..50, default 13. higher = subtler transformation
  --algorithm NAME            genetic (default) or optimal
  --max-seconds N             stop the solve after N seconds, keeping the best
                              assignment found so far (default: run to convergence)
  --assignment PATH           the assignment.json to render
  --before-after              render source and result side by side
  --scale N                   pixels per cell in output images, default 4
  --rule RRGGBB               before/after divider color, default 1c1c1c
  -h, --help                  this text

examples:
  labronify reduce --in face.jpg --out face.128.png --side 128
  labronify solve  --in face.jpg --out face.json --side 128 --proximity 13
  labronify render --in face.jpg --assignment face.json --out face.after.png
  labronify render --in face.jpg --assignment face.json --out ba.png --before-after
  labronify export --in notes.txt --assignment notes.json --out moved.txt --side 128
";

#[derive(Clone, Debug)]
pub struct Options {
    pub command: Command,
    /// Pixels per cell in rendered images.
    pub scale: u32,
    /// Divider color for before/after strips.
    pub rule_rgb: [u8; 3],
}

#[derive(Clone, Debug)]
pub enum Command {
    Tiers,
    Reduce {
        input: PathBuf,
        out: PathBuf,
        side: u32,
        crop: CropScale,
    },
    Solve {
        input: PathBuf,
        out: PathBuf,
        side: u32,
        crop: CropScale,
        target_crop: CropScale,
        proximity_importance: i64,
        algorithm: Algorithm,
        max_seconds: Option<u64>,
    },
    Render {
        input: PathBuf,
        assignment: PathBuf,
        out: PathBuf,
        side: u32,
        crop: CropScale,
        before_after: bool,
    },
    Export {
        input: PathBuf,
        assignment: PathBuf,
        out: PathBuf,
        side: u32,
        crop: CropScale,
    },
}

pub enum Parsed {
    Help,
    Command(Options),
}

/// Small helper so every "you forgot a value" error reads the same way.
struct Cursor<'a> {
    argv: &'a [String],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn new(argv: &'a [String]) -> Self {
        Self { argv, i: 0 }
    }

    fn next(&mut self) -> Option<&'a str> {
        let v = self.argv.get(self.i)?;
        self.i += 1;
        Some(v.as_str())
    }

    /// The value for the flag we just consumed.
    fn value(&mut self, flag: &str) -> Result<&'a str, String> {
        match self.argv.get(self.i) {
            Some(v) if !v.starts_with("--") => {
                self.i += 1;
                Ok(v.as_str())
            }
            _ => Err(format!("{flag} needs a value")),
        }
    }
}

fn parse_crop(flag: &str, raw: &str) -> Result<CropScale, String> {
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 3 {
        return Err(format!(
            "{flag} expects three comma-separated numbers like 0,0,1, got {raw:?}"
        ));
    }
    let num = |s: &str| -> Result<f32, String> {
        s.trim()
            .parse::<f32>()
            .map_err(|_| format!("{flag}: {s:?} is not a number"))
    };
    let crop = CropScale {
        x: num(parts[0])?,
        y: num(parts[1])?,
        scale: num(parts[2])?,
    };
    let (lo, hi) = lab_core::CROP_OFFSET_RANGE;
    let (slo, shi) = lab_core::CROP_SCALE_RANGE;
    if !(lo..=hi).contains(&crop.x) || !(lo..=hi).contains(&crop.y) {
        return Err(format!("{flag}: x and y must be in {lo}..{hi}"));
    }
    if !(slo..=shi).contains(&crop.scale) {
        return Err(format!("{flag}: scale must be in {slo}..{shi}"));
    }
    Ok(crop)
}

fn parse_rule(raw: &str) -> Result<[u8; 3], String> {
    let raw = raw.trim_start_matches('#');
    if raw.len() != 6 {
        return Err(format!(
            "--rule expects 6 hex digits like 1c1c1c, got {raw:?}"
        ));
    }
    let byte = |i: usize| -> Result<u8, String> {
        u8::from_str_radix(&raw[i..i + 2], 16)
            .map_err(|_| format!("--rule: {raw:?} is not valid hex"))
    };
    Ok([byte(0)?, byte(2)?, byte(4)?])
}
pub fn parse(argv: &[String]) -> Result<Parsed, String> {
    let mut cur = Cursor::new(argv);

    let command = match cur.next() {
        Some("-h") | Some("--help") | Some("help") | None => return Ok(Parsed::Help),
        Some(c) => c.to_string(),
    };

    // Shared defaults, overridable by any command.
    let mut side: Option<u32> = None;
    let mut crop = CropScale::identity();
    let mut target_crop = CropScale::identity();
    let mut proximity: Option<i64> = None;
    let mut algorithm = Algorithm::Genetic;
    let mut max_seconds: Option<u64> = None;
    let mut scale = lab_export::DEFAULT_PIXEL_SCALE;
    let mut rule_rgb = [0x1c, 0x1c, 0x1c];

    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut assignment: Option<PathBuf> = None;
    let mut before_after = false;

    while let Some(flag) = cur.next() {
        match flag {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--in" => input = Some(PathBuf::from(cur.value("--in")?)),
            "--out" => out = Some(PathBuf::from(cur.value("--out")?)),
            "--assignment" => assignment = Some(PathBuf::from(cur.value("--assignment")?)),
            "--before-after" => before_after = true,
            "--side" => {
                let raw = cur.value("--side")?;
                let n: u32 = raw
                    .parse()
                    .map_err(|_| format!("--side expects an integer, got {raw:?}"))?;
                let (lo, hi) = lab_core::RESOLUTION_RANGE;
                if !(lo..=hi).contains(&n) {
                    return Err(format!(
                        "--side must be in {lo}..{hi}; that is the range the cost model is \
                         calibrated for"
                    ));
                }
                side = Some(n);
            }
            "--crop" => crop = parse_crop("--crop", cur.value("--crop")?)?,
            "--target-crop" => {
                target_crop = parse_crop("--target-crop", cur.value("--target-crop")?)?
            }
            "--proximity" => {
                let raw = cur.value("--proximity")?;
                let n: i64 = raw
                    .parse()
                    .map_err(|_| format!("--proximity expects an integer, got {raw:?}"))?;
                let (lo, hi) = lab_core::PROXIMITY_IMPORTANCE_RANGE;
                if !(lo..=hi).contains(&n) {
                    return Err(format!("--proximity must be in {lo}..{hi}"));
                }
                proximity = Some(n);
            }
            "--algorithm" => {
                let raw = cur.value("--algorithm")?;
                algorithm = match raw {
                    "genetic" => Algorithm::Genetic,
                    "optimal" => Algorithm::Optimal,
                    other => {
                        return Err(format!(
                            "--algorithm must be genetic or optimal, got {other:?}"
                        ))
                    }
                };
            }
            "--max-seconds" => {
                let raw = cur.value("--max-seconds")?;
                let n: u64 = raw
                    .parse()
                    .map_err(|_| format!("--max-seconds expects an integer, got {raw:?}"))?;
                if n == 0 {
                    return Err("--max-seconds must be at least 1".into());
                }
                max_seconds = Some(n);
            }
            "--scale" => {
                let raw = cur.value("--scale")?;
                let n: u32 = raw
                    .parse()
                    .map_err(|_| format!("--scale expects an integer, got {raw:?}"))?;
                if n == 0 {
                    return Err("--scale must be at least 1".into());
                }
                scale = n;
            }
            "--rule" => rule_rgb = parse_rule(cur.value("--rule")?)?,
            other => {
                if other.starts_with("--") {
                    return Err(format!("unknown option {other:?}"));
                }
                return Err(format!(
                    "unexpected argument {other:?}; the command goes first, then its options"
                ));
            }
        }
    }

    let side = side.unwrap_or(lab_core::CALIBRATION_SIDELEN);
    let proximity = proximity.unwrap_or(lab_core::DEFAULT_PROXIMITY_IMPORTANCE);

    let need_in = |p: &Option<PathBuf>| -> Result<PathBuf, String> {
        p.clone().ok_or_else(|| "--in is required".to_string())
    };
    let need_out = |p: &Option<PathBuf>| -> Result<PathBuf, String> {
        p.clone().ok_or_else(|| "--out is required".to_string())
    };

    let command = match command.as_str() {
        "tiers" | "tier" => Command::Tiers,
        "reduce" => Command::Reduce {
            input: need_in(&input)?,
            out: need_out(&out)?,
            side,
            crop,
        },
        "solve" => Command::Solve {
            input: need_in(&input)?,
            out: need_out(&out)?,
            side,
            crop,
            target_crop,
            proximity_importance: proximity,
            algorithm,
            max_seconds,
        },
        "render" => Command::Render {
            input: need_in(&input)?,
            assignment: assignment.ok_or_else(|| "--assignment is required".to_string())?,
            out: need_out(&out)?,
            side,
            crop,
            before_after,
        },
        "export" => Command::Export {
            input: need_in(&input)?,
            assignment: assignment.ok_or_else(|| "--assignment is required".to_string())?,
            out: need_out(&out)?,
            side,
            crop,
        },
        other => {
            return Err(format!(
                "unknown command {other:?}; try reduce, solve, render, export or tiers"
            ))
        }
    };

    Ok(Parsed::Command(Options {
        command,
        scale,
        rule_rgb,
    }))
}
