//! Code reduction: source becomes a syntax-colored texture.
//!
//! One character is one pixel, so the picture keeps the shape of the file: line
//! lengths, indentation and comment blocks all survive. Color carries the token
//! class, using the common dark-editor scheme, so a keyword, a string and a
//! number are visibly different regions rather than the same gray. Lines longer
//! than the canvas wrap instead of being cut, and the scan stops at a total
//! character budget so a huge file cannot outgrow the canvas.

use image::{Rgb, RgbImage};
use lab_core::medium::Medium;
use lab_core::reducer::{ReduceError, Reducer, Tier};

use crate::finish_canvas;

const MAX_ROWS: usize = 512;
const MAX_COLS: usize = 512;

const BG: Rgb<u8> = Rgb([30, 30, 30]);
const PLAIN: Rgb<u8> = Rgb([212, 212, 212]);
const KEYWORD: Rgb<u8> = Rgb([197, 134, 192]);
const STRING: Rgb<u8> = Rgb([206, 145, 120]);
const NUMBER: Rgb<u8> = Rgb([181, 206, 168]);
const COMMENT: Rgb<u8> = Rgb([106, 153, 85]);
const IDENT: Rgb<u8> = Rgb([156, 220, 254]);

/// Keywords across the languages the tier claims. A word that is a keyword in
/// one language and an identifier in another is colored as a keyword either
/// way: the reduction cannot know the language from the bytes, and the coloring
/// only has to be consistent, not exact.
const KEYWORDS: &[&str] = &[
    "fn",
    "let",
    "mut",
    "const",
    "if",
    "else",
    "for",
    "while",
    "loop",
    "match",
    "return",
    "struct",
    "enum",
    "impl",
    "trait",
    "pub",
    "use",
    "mod",
    "self",
    "Self",
    "true",
    "false",
    "None",
    "Some",
    "def",
    "class",
    "import",
    "from",
    "in",
    "is",
    "not",
    "and",
    "or",
    "elif",
    "try",
    "except",
    "finally",
    "with",
    "as",
    "lambda",
    "yield",
    "async",
    "await",
    "int",
    "float",
    "str",
    "bool",
    "func",
    "var",
    "package",
    "type",
    "interface",
    "go",
    "defer",
    "range",
    "switch",
    "case",
    "break",
    "continue",
    "new",
    "delete",
    "void",
    "static",
    "public",
    "private",
    "final",
    "this",
    "null",
    "undefined",
    "throw",
    "catch",
    "do",
    "using",
    "namespace",
    "record",
];

/// The code tier: colorize, then let the engine downscale the character grid.
#[derive(Clone, Debug, Default)]
pub struct CodeReducer {
    pub crop: lab_core::CropScale,
}

impl CodeReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for CodeReducer {
    fn tier(&self) -> Tier {
        Tier::Code
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Code));
        }
        let text = String::from_utf8_lossy(input);
        let rows = colorize(&text);
        if rows.iter().flatten().all(|(_, c)| *c == BG) {
            return Err(ReduceError::Empty(Tier::Code));
        }
        let w = rows.iter().map(Vec::len).max().unwrap_or(0).max(1) as u32;
        let h = rows.len().max(1) as u32;
        let mut img = RgbImage::from_pixel(w, h, BG);
        for (y, row) in rows.iter().enumerate() {
            for (x, (_, c)) in row.iter().enumerate() {
                img.put_pixel(x as u32, y as u32, *c);
            }
        }
        finish_canvas(Tier::Code, img, &self.crop, side)
    }
}

/// The characters laid out as the reducer lays them out, for the export.
pub(crate) fn char_rows(input: &[u8]) -> Result<Vec<Vec<char>>, ReduceError> {
    if input.is_empty() {
        return Err(ReduceError::Empty(Tier::Code));
    }
    let rows = colorize(&String::from_utf8_lossy(input));
    if rows.iter().flatten().all(|(_, c)| *c == BG) {
        return Err(ReduceError::Empty(Tier::Code));
    }
    Ok(rows
        .iter()
        .map(|r| r.iter().map(|(c, _)| *c).collect())
        .collect())
}

/// One row per line; each row is one character and its color. The character is
/// kept so the export can move the real source instead of a colored shadow.
pub(crate) fn colorize(text: &str) -> Vec<Vec<(char, Rgb<u8>)>> {
    let chars: Vec<char> = text.chars().collect();
    let budget = MAX_ROWS * MAX_COLS;
    let mut rows: Vec<Vec<(char, Rgb<u8>)>> = vec![Vec::new()];
    let mut produced = 0usize;
    let mut in_line = false;
    let mut in_block = false;
    let mut i = 0usize;

    while i < chars.len() && produced < budget && rows.len() <= MAX_ROWS {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        if in_line {
            if c == '\n' {
                in_line = false;
                rows.push(Vec::new());
            } else {
                push(&mut rows, c, COMMENT, &mut produced);
            }
            i += 1;
            continue;
        }
        if in_block {
            if c == '*' && next == Some('/') {
                push(&mut rows, '*', COMMENT, &mut produced);
                push(&mut rows, '/', COMMENT, &mut produced);
                in_block = false;
                i += 2;
            } else {
                if c == '\n' {
                    rows.push(Vec::new());
                } else {
                    push(&mut rows, c, COMMENT, &mut produced);
                }
                i += 1;
            }
            continue;
        }

        if c == '/' && next == Some('/') {
            push(&mut rows, c, COMMENT, &mut produced);
            in_line = true;
            i += 1;
        } else if c == '/' && next == Some('*') {
            push(&mut rows, c, COMMENT, &mut produced);
            in_block = true;
            i += 1;
        } else if c == '#' {
            push(&mut rows, c, COMMENT, &mut produced);
            in_line = true;
            i += 1;
        } else if c == '"' || c == '\'' || c == '`' {
            i = scan_string(&chars, i, &mut rows, &mut produced);
        } else if c.is_ascii_digit() {
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                push(&mut rows, chars[i], NUMBER, &mut produced);
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let color = if KEYWORDS.contains(&word.as_str()) {
                KEYWORD
            } else {
                IDENT
            };
            for &ch in &chars[start..i] {
                push(&mut rows, ch, color, &mut produced);
            }
        } else if c == '\n' {
            rows.push(Vec::new());
            i += 1;
        } else if c == '\t' {
            for _ in 0..4 {
                push(&mut rows, ' ', BG, &mut produced);
            }
            i += 1;
        } else if c == ' ' {
            push(&mut rows, c, BG, &mut produced);
            i += 1;
        } else {
            push(&mut rows, c, PLAIN, &mut produced);
            i += 1;
        }
    }

    while rows.len() > 1 && rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }
    rows
}

/// Consume a string literal starting at `start`, returning the next index.
fn scan_string(
    chars: &[char],
    start: usize,
    rows: &mut Vec<Vec<(char, Rgb<u8>)>>,
    produced: &mut usize,
) -> usize {
    let quote = chars[start];
    let mut i = start;
    push(rows, chars[i], STRING, produced);
    i += 1;
    while i < chars.len() {
        let c = chars[i];
        push(rows, c, STRING, produced);
        i += 1;
        if c == '\\' {
            if i < chars.len() {
                push(rows, chars[i], STRING, produced);
                i += 1;
            }
        } else if c == quote || (c == '\n' && quote != '`') {
            break;
        }
    }
    i
}

/// Append one character and its color, wrapping to a new line at the canvas width.
fn push(rows: &mut Vec<Vec<(char, Rgb<u8>)>>, c: char, color: Rgb<u8>, produced: &mut usize) {
    if rows.last().is_none_or(|r| r.len() >= MAX_COLS) {
        rows.push(Vec::new());
    }
    rows.last_mut().unwrap().push((c, color));
    *produced += 1;
}
