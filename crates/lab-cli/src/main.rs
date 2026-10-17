//! The labronify command line.
//!
//! Four commands, because the pipeline has four stages and each one should be
//! independently runnable and inspectable:
//!
//! * `reduce`  medium bytes -> the N x N grid the engine actually sees
//! * `solve`   the grid + the LeBron target -> `assignment.json`
//! * `render`  `assignment.json` + the grid -> pixels
//! * `export`  `assignment.json` + the grid -> the medium itself, cells moved
//!
//! `solve` is the only expensive step, so its output is the artifact; `render`
//! and `export` are two answers to the same artifact, a picture and the medium.
//!
//! Argument parsing lives in `args`; see the crate `Cargo.toml` for why there is
//! no argument-parsing dependency.

mod args;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{Command, Options};

use lab_core::reducer::TIERS;
use lab_core::{Artifact, Assignment, CropScale, Settings};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    let opts = match args::parse(&argv) {
        Ok(args::Parsed::Help) => {
            println!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(args::Parsed::Command(o)) => o,
        Err(e) => {
            eprintln!("error: {e}\n");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(2);
        }
    };

    match run(&opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(opts: &Options) -> Result<(), Box<dyn std::error::Error>> {
    match &opts.command {
        Command::Tiers => {
            print_tiers();
            Ok(())
        }

        Command::Reduce {
            input,
            out,
            side,
            crop,
        } => {
            let bytes = read(input)?;
            let medium = reduce_medium(input, &bytes, *side, *crop)?;
            let png = lab_export::medium_to_png_scaled(&medium, opts.scale)?;
            write(out, &png)?;
            eprintln!(
                "reduced {} ({} bytes) to a {}x{} grid, wrote {}",
                input.display(),
                bytes.len(),
                side,
                side,
                out.display()
            );
            Ok(())
        }

        Command::Solve {
            input,
            out,
            side,
            crop,
            target_crop,
            proximity_importance,
            algorithm,
            max_seconds,
        } => {
            let bytes = read(input)?;
            let source = reduce_medium(input, &bytes, *side, *crop)?;

            // Match the reference app: the proximity slider is calibrated at
            // solve time, then the crops are stored exactly as given.
            let mut settings = Settings {
                sidelen: *side,
                proximity_importance: *proximity_importance,
                algorithm: *algorithm,
                source_crop: *crop,
                target_crop: *target_crop,
            }
            .calibrated();
            settings.source_crop = *crop;
            settings.target_crop = *target_crop;

            let (target, weights) = lab_core::target_and_weights(&settings)?;

            let started = std::time::Instant::now();
            // An optional budget: the solver polls this every 1<<16 proposals and
            // returns the best assignment so far, labelled `Stopped`. No budget
            // means run to the reference convergence rule, unchanged.
            let deadline = max_seconds.map(|s| started + std::time::Duration::from_secs(s));
            let should_stop = || deadline.is_some_and(|d| std::time::Instant::now() >= d);
            let solution = lab_core::solve(
                &source,
                &target,
                &weights,
                settings.proximity_importance,
                settings.algorithm,
                &should_stop,
            );
            let assignment = Assignment::from(solution.clone());

            let artifact = Artifact::new(settings.clone(), &assignment);
            write(out, artifact.to_json().as_bytes())?;

            eprintln!(
                "solved {} cells with {:?} in {:.2}s ({} generation(s), {:?}), cost {}",
                assignment.len(),
                settings.algorithm,
                started.elapsed().as_secs_f64(),
                solution.generations,
                solution.outcome,
                solution.cost
            );
            eprintln!("wrote {}", out.display());
            Ok(())
        }

        Command::Render {
            input,
            assignment,
            out,
            side,
            crop,
            before_after,
        } => render_command(opts, input, assignment, out, *side, *crop, *before_after),

        Command::Export {
            input,
            assignment,
            out,
            side,
            crop,
        } => export_command(input, assignment, out, *side, *crop),
    }
}

/// Write the source in its own format, with the cells of `assignment` moved.
fn export_command(
    input: &PathBuf,
    assignment: &PathBuf,
    out: &PathBuf,
    side: u32,
    crop: CropScale,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = read(input)?;
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let tier = lab_core::reducer::tier_for_extension(&ext)
        .ok_or_else(|| format!("no tier handles {ext:?} files"))?;

    let artifact = load_artifact(&read_to_string(assignment)?, side)?;
    let loaded = artifact.to_assignment()?;
    if loaded.side() != side {
        return Err(format!(
            "the assignment is for a {}x{} grid but --side is {side}; \
             re-run solve with the same --side",
            loaded.side(),
            loaded.side()
        )
        .into());
    }

    match lab_media::export_native(tier, &bytes, &loaded, crop)? {
        Some(native) => {
            write(out, &native)?;
            eprintln!(
                "exported the {} tier to {} with its cells moved",
                tier.id(),
                out.display()
            );
            Ok(())
        }
        None => Err(format!(
            "the {} tier has no native export; use render for a picture",
            tier.id()
        )
        .into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_command(
    opts: &Options,
    input: &PathBuf,
    assignment: &PathBuf,
    out: &PathBuf,
    side: u32,
    crop: CropScale,
    before_after: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = read(input)?;
    let source = reduce_medium(input, &bytes, side, crop)?;

    let json = read_to_string(assignment)?;
    let artifact = load_artifact(&json, side)?;
    let loaded = artifact.to_assignment()?;

    if loaded.side() != source.side() {
        return Err(format!(
            "the assignment is for a {}x{} grid but the source reduced to {}x{}; \
             re-run solve with the same --side",
            loaded.side(),
            loaded.side(),
            source.side(),
            source.side()
        )
        .into());
    }

    let png = if before_after {
        let after = lab_core::render(&source, &loaded);
        lab_export::before_after_png(&source, &after, opts.scale, opts.rule_rgb)?
    } else {
        lab_export::render_png(&source, &loaded, opts.scale)?
    };
    write(out, &png)?;

    if before_after {
        eprintln!("wrote before/after {} (source | result)", out.display());
    } else {
        eprintln!("wrote {}", out.display());
    }
    Ok(())
}

/// Decode `assignment.json`, accepting both labronify's artifact and a bare
/// permutation array (which is what the reference app writes).
fn load_artifact(json: &str, side: u32) -> Result<Artifact, Box<dyn std::error::Error>> {
    if json.trim_start().starts_with('[') {
        return Artifact::from_bare_array(side, json).map_err(|e| e.to_string().into());
    }
    Artifact::from_json(json).map_err(|e| e.to_string().into())
}

fn reduce_medium(
    input: &Path,
    bytes: &[u8],
    side: u32,
    crop: CropScale,
) -> Result<lab_core::Medium, Box<dyn std::error::Error>> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(lab_media::reduce_file(bytes, &ext, side, crop)?)
}

fn print_tiers() {
    // Same columns as the generated docs/MEDIA-MATRIX.md, so the two cannot
    // disagree about what exists.
    println!(
        "{:<5} {:<11} {:<6} {:<28} reduction",
        "tier", "id", "state", "extensions"
    );
    for spec in TIERS {
        println!(
            "{:<5} {:<11} {:<6} {:<28} {}",
            spec.tier.label(),
            spec.tier.id(),
            spec.state.glyph(),
            spec.extensions.join(","),
            spec.tier.reduction()
        );
    }
    println!();
    println!(
        "state: ok = wired into the CLI, wip = reducer exists but not wired, -- = design only"
    );
    println!("`cargo xtask matrix --check` fails the build when that table drifts");
}

fn read(path: &PathBuf) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()).into())
}

fn read_to_string(path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()).into())
}

fn write(path: &PathBuf, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() && !dir.exists() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
    }
    std::fs::write(path, bytes)
        .map_err(|e| format!("could not write {}: {e}", path.display()).into())
}
