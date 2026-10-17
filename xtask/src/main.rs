//! Build tasks.
//!
//! Two jobs, both of which exist to keep a promise the project makes:
//!
//! * `guard`  scans shipped source and fails if anything could reach the network
//! * `matrix` regenerates `docs/MEDIA-MATRIX.md` from the code
//!
//! Run with `cargo xtask <task>`. `--check` makes a task read-only and exits
//! non-zero on drift, which is the form CI uses.

mod guard;
mod matrix;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <task> [--check]

tasks:
  guard           fail if any shipped source could reach the network
  guard --list    print what the guard forbids and what it skips
  matrix          regenerate docs/MEDIA-MATRIX.md from the tier table
  matrix --check  fail if docs/MEDIA-MATRIX.md is out of date
  test            the guard and the matrix check, i.e. what CI runs first
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let list = args.iter().any(|a| a == "--list");
    let task = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .unwrap_or("");

    let root = match workspace_root() {
        Some(r) => r,
        None => {
            eprintln!(
                "error: could not locate the workspace root from {}",
                env!("CARGO_MANIFEST_DIR")
            );
            return ExitCode::FAILURE;
        }
    };

    match task {
        "" => {
            print!("{USAGE}");
            ExitCode::from(2)
        }
        "guard" => {
            if list {
                print_guard_policy();
                return ExitCode::SUCCESS;
            }
            let report = guard::scan(&root);
            print!("{}", guard::format_report(&report));
            if report.is_clean() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "matrix" => {
            let generated = matrix::render();
            if check {
                match matrix::check_on_disk(&root) {
                    Ok(()) => {
                        println!("media matrix: up to date");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                let path = root.join("docs").join("MEDIA-MATRIX.md");
                if let Some(dir) = path.parent() {
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        eprintln!("error: could not create {}: {e}", dir.display());
                        return ExitCode::FAILURE;
                    }
                }
                match std::fs::write(&path, generated) {
                    Ok(()) => {
                        println!("wrote {}", path.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: could not write {}: {e}", path.display());
                        ExitCode::FAILURE
                    }
                }
            }
        }
        "test" => {
            let mut failed = false;

            let report = guard::scan(&root);
            print!("{}", guard::format_report(&report));
            failed |= !report.is_clean();

            match matrix::check_on_disk(&root) {
                Ok(()) => println!("media matrix: up to date"),
                Err(e) => {
                    eprintln!("error: {e}");
                    failed = true;
                }
            }

            if failed {
                ExitCode::FAILURE
            } else {
                println!("\nall xtask checks passed");
                ExitCode::SUCCESS
            }
        }
        other => {
            eprintln!("error: unknown task {other:?}\n");
            print!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The workspace root, found by walking up from this crate until a `Cargo.toml`
/// with a `[workspace]` section is found.
fn workspace_root() -> Option<PathBuf> {
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    for _ in 0..5 {
        let manifest = dir.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if text.contains("[workspace]") {
                return Some(dir);
            }
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

fn print_guard_policy() {
    println!("forbidden tokens (case-insensitive, matched in source text):");
    for t in guard::FORBIDDEN {
        println!("  {t}");
    }
    println!("\nforbidden `fetch` forms (the bare word is allowed in prose):");
    for t in guard::FORBIDDEN_FETCH_FORMS {
        println!("  {t}");
    }
    println!("\nexempt path prefixes (not shipped):");
    for t in guard::EXEMPT_PATHS {
        println!("  {t}");
    }
    println!("\nscanned file extensions:");
    println!("  {}", guard::SCANNED_EXTENSIONS.join(" "));
}
