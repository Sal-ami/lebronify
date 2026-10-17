//! The privacy guard.
//!
//! labronify's whole premise is that nothing leaves the device. That promise is
//! enforced here rather than trusted: this module scans every source file the
//! project ships and fails the build if any of them can open a network
//! connection, directly or through a call the browser performs on a page's
//! behalf.
//!
//! The rule is deliberately stricter than "no networking in the offline core". Any
//! of these appearing in shipped code is a review-worthy event even if it is
//! currently unreachable, because the cheap way for networking to sneak back in
//! is a helper function that is wired up later.
//!
//! There is one intentional hole: reference material is exempt, because the
//! vendored upstream app is a *developing aid*, not shipped code. Exemptions are
//! listed explicitly in [`EXEMPT_PATHS`] so adding one is a visible decision.

use std::fs;
use std::path::{Path, PathBuf};

/// Substrings that must not appear in shipped source.
///
/// Matched case-insensitively against the *source text*, so a comment that says
/// "we never call fetch here" also trips the guard. That is intended: it keeps the
/// vocabulary out of the codebase, and the fix (rephrase the comment) is trivial.
pub const FORBIDDEN: &[&str] = &[
    "xmlhttprequest",
    "websocket",
    "sendbeacon",
    "eventsource",
    "navigator.send",
    "reqwest",
    "ureq::",
    "hyper::client",
    "tokio::net",
    "std::net::",
    "curl ",
    "wget ",
    "importscripts",
];

/// `fetch` needs care: it is a common substring in ordinary words ("fetching",
/// "prefetch"). Only these exact forms are forbidden.
///
/// These deliberately overlap: `window.fetch(` matches three of them. Reports are
/// deduplicated per line, so overlap costs nothing but keeps each pattern
/// readable on its own.
pub const FORBIDDEN_FETCH_FORMS: &[&str] =
    &["fetch(", ".fetch(", "window.fetch", "globalthis.fetch"];

/// Paths (as prefixes, relative to the workspace root) that the guard skips.
pub const EXEMPT_PATHS: &[&str] = &[
    // Vendored upstream reference material. Not built, not shipped.
    "reference/",
    // The guard's own source necessarily contains the forbidden words.
    "xtask/src/guard.rs",
    // Docs explain the policy and must be able to name what is banned.
    "docs/",
    // Build scratch, never shipped.
    "target/",
];

/// Extensions the guard reads. Anything else is skipped rather than guessed at.
pub const SCANNED_EXTENSIONS: &[&str] = &[
    "rs", "toml", "json", "js", "ts", "html", "htm", "css", "wgsl", "sh", "ps1", "psm1", "yml",
    "yaml", "sql", "svg", "md",
];

/// One offending line. A line is reported once no matter how many patterns it
/// matched: three overlapping `fetch` patterns describing one call is one finding,
/// not three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub path: PathBuf,
    pub line: usize,
    /// Every forbidden token that matched on this line, in declaration order.
    pub tokens: Vec<String>,
    /// The offending line, trimmed, for the error message.
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardReport {
    pub files_scanned: usize,
    pub violations: Vec<Violation>,
}

impl GuardReport {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// True when `rel` (a workspace-relative path with `/` separators) is exempt.
pub fn is_exempt(rel: &str) -> bool {
    EXEMPT_PATHS.iter().any(|p| rel.starts_with(p))
}

/// Scan the workspace rooted at `root`.
pub fn scan(root: &Path) -> GuardReport {
    let mut report = GuardReport {
        files_scanned: 0,
        violations: Vec::new(),
    };
    walk(root, root, &mut report);
    report
}

fn walk(root: &Path, dir: &Path, report: &mut GuardReport) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        // An unreadable directory is not a privacy violation, and not something
        // the guard can fix, so it is skipped rather than fatal.
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let rel = relative(root, &path);
        if is_exempt(&rel) {
            continue;
        }

        if file_type.is_dir() {
            walk(root, &path, report);
            continue;
        }

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !SCANNED_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        report.files_scanned += 1;
        scan_text(&rel, &text, report);
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn scan_text(rel: &str, text: &str, report: &mut GuardReport) {
    for (index, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();

        let mut tokens = Vec::new();
        for token in FORBIDDEN.iter().chain(FORBIDDEN_FETCH_FORMS.iter()) {
            if lower.contains(token) {
                tokens.push((*token).to_string());
            }
        }

        if !tokens.is_empty() {
            report.violations.push(Violation {
                path: PathBuf::from(rel),
                line: index + 1,
                tokens,
                snippet: line.trim().to_string(),
            });
        }
    }
}

/// Render a report as the human-readable failure a reviewer links to.
pub fn format_report(report: &GuardReport) -> String {
    if report.is_clean() {
        return format!(
            "no-network guard: clean ({} file(s) scanned)\n",
            report.files_scanned
        );
    }

    let mut out = String::new();
    out.push_str(&format!(
        "no-network guard: {} violation(s) in {} scanned file(s)\n\n",
        report.violations.len(),
        report.files_scanned
    ));
    out.push_str(
        "labronify promises that nothing leaves the device. Shipped code must not be\n\
         able to open a network connection. If a violation is a false positive, tighten\n\
         the pattern above or add a path prefix to EXEMPT_PATHS; both are visible review\n\
         decisions, which is the point.\n\n",
    );
    for v in &report.violations {
        out.push_str(&format!(
            "  {}:{}  matched {}\n      {}\n",
            v.path.display(),
            v.line,
            v.tokens
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            v.snippet
        ));
    }
    out
}
