//! Website reduction: a page's box structure as an N x N picture.
//!
//! A web page is not a picture, but it does have a layout, and layout is
//! two-dimensional. This reducer parses the markup, works out what the page
//! would look like as blocks, and paints them into the cell grid.
//!
//! What it is, stated plainly so nobody expects more: a *box-model sketch*, not
//! a render. There is no CSS cascade, no floats, no flexbox, no fonts, and no
//! network - external stylesheets, images and scripts are deliberately not
//! fetched, both because the program promises to stay offline and because the
//! reducer contract gives it bytes, not a URL. It uses inline `style`
//! attributes, presentational tags and defaults, which is enough to give each
//! page a picture of its own that is stable and recognisably derived from it.
//!
//! One structural rule makes the output much better than a flat dump: elements
//! that sit side by side are laid out side by side. A nav bar and a body
//! paragraph produce two columns, not two stacked rows, which is what a person
//! sees when they look at the page.

use lab_core::medium::{Medium, Pixel};
use lab_core::reducer::{ReduceError, Reducer, Tier};

/// Pixels in the abstract viewport the page is laid out into.
pub const VIEWPORT_W: f64 = 1000.0;
/// Pixels in the abstract viewport's height. A page taller than this is scaled
/// down rather than truncated, so a long article still shows its whole flow.
pub const VIEWPORT_H: f64 = 1400.0;
/// Cap on how many elements are painted, so a pathological document cannot make
/// the reducer slow. Layout keeps going; only painting stops.
const MAX_BOXES: usize = 20_000;

/// One painted region of the page, in viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2d {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub color: (u8, u8, u8),
    /// Earlier boxes are painted first, so later ones land on top.
    pub order: u32,
}

/// Block tags whose children are laid out top to bottom.
fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "html"
            | "head"
            | "body"
            | "div"
            | "p"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "form"
            | "fieldset"
            | "figure"
            | "figcaption"
            | "blockquote"
            | "pre"
            | "details"
            | "summary"
            | "address"
            | "title"
            | "meta"
            | "link"
            | "script"
            | "style"
    )
}

/// Elements with no visual presence of their own. Their whole subtree is
/// unpaintable: a `<title>` inside `<head>` is as invisible as the head.
fn is_invisible(tag: &str) -> bool {
    matches!(tag, "head" | "title" | "meta" | "link" | "script" | "style")
}

/// A child that shares its parent's row instead of starting a new one.
///
/// Inline elements always do. A block element does too once it has an explicit
/// width, which is how a browser lays out `<div style="width:50%">` pairs side
/// by side. Without this a two-column page collapses into one column.
fn is_row_item(tag: &str, style: &Style) -> bool {
    !is_block(tag) || style.width_pct.is_some()
}

/// The color painted when a tag has no color of its own and no explicit style.
const DEFAULT_TAG_COLOR: (u8, u8, u8) = (128, 128, 128);

/// The default paint color for a tag.
fn tag_color(tag: &str) -> (u8, u8, u8) {
    match tag {
        "h1" => (236, 236, 236),
        "h2" => (212, 212, 212),
        "h3" => (188, 188, 188),
        "h4" | "h5" | "h6" => (168, 168, 168),
        "a" => (64, 132, 220),
        "button" | "input" | "select" | "textarea" => (150, 150, 158),
        "code" | "pre" | "kbd" | "samp" => (58, 62, 72),
        "li" => (176, 176, 176),
        "td" => (196, 196, 196),
        "th" => (218, 218, 218),
        "hr" => (100, 100, 100),
        "img" | "svg" | "video" | "canvas" | "picture" => (36, 92, 74),
        "table" => (160, 160, 168),
        "blockquote" => (140, 140, 150),
        "footer" => (92, 92, 100),
        "header" => (108, 108, 116),
        "nav" => (84, 84, 96),
        "body" => (242, 242, 242),
        "html" => (250, 250, 250),
        _ => DEFAULT_TAG_COLOR,
    }
}

/// Resolve a CSS color literal: `#rgb`, `#rrggbb`, or a name from the small set
/// that covers most presentational markup.
fn parse_css_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let nib = |c: u8| (c as char).to_digit(16);
        let bytes = hex.as_bytes();
        return match bytes.len() {
            3 => {
                let (r, g, b) = (nib(bytes[0])?, nib(bytes[1])?, nib(bytes[2])?);
                Some(((r * 17) as u8, (g * 17) as u8, (b * 17) as u8))
            }
            6 => {
                let pair = |i: usize| -> Option<u8> {
                    Some((nib(bytes[i])? * 16 + nib(bytes[i + 1])?) as u8)
                };
                Some((pair(0)?, pair(2)?, pair(4)?))
            }
            _ => None,
        };
    }
    Some(match s.as_str() {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (200, 30, 30),
        "green" => (30, 140, 60),
        "lime" => (140, 220, 40),
        "blue" => (40, 80, 200),
        "yellow" => (230, 220, 60),
        "orange" => (240, 140, 40),
        "purple" => (130, 60, 180),
        "pink" => (240, 130, 180),
        "brown" => (130, 90, 60),
        "gray" | "grey" => (128, 128, 128),
        "silver" => (192, 192, 192),
        "navy" => (20, 30, 100),
        "teal" => (30, 130, 130),
        "olive" => (130, 130, 40),
        "maroon" => (130, 30, 40),
        "gold" => (230, 190, 40),
        "cyan" | "aqua" => (40, 200, 220),
        "magenta" | "fuchsia" => (220, 40, 200),
        _ => return None,
    })
}

/// The subset of presentation this reducer understands. Deliberately small: it
/// reads inline styles and presentational attributes, and nothing else.
#[derive(Clone, Copy, Debug, Default)]
struct Style {
    /// Explicit `height: Npx`.
    explicit_h: Option<f64>,
    /// Explicit `width: N%` of the parent's content box.
    width_pct: Option<f64>,
    /// Uniform padding in pixels.
    pad: f64,
    /// Space inserted above the element in its parent's flow.
    margin_top: f64,
    /// `color` / `background-color`, or a presentational attribute.
    color: Option<(u8, u8, u8)>,
    /// `display: inline`, or an `inline-block`; either way it sits in a row.
    inline: bool,
}

/// Read a `NNpx` length. Unitless numbers are accepted as pixels, which is what
/// a browser does for the properties used here.
fn px(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches("px").trim();
    s.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

/// Parse an element's attributes for the presentation this reducer uses.
fn parse_style(attrs: &str) -> Style {
    let lower = attrs.to_ascii_lowercase();
    let mut style = Style {
        // Everything with text in it needs a minimum height or it would paint
        // as a zero-height sliver and vanish.
        explicit_h: None,
        ..Default::default()
    };

    // Legacy presentational attributes.
    if let Some(c) = attr_value(&lower, "bgcolor").and_then(parse_css_color) {
        style.color = Some(c);
    }
    if let Some(c) = attr_value(&lower, "color").and_then(parse_css_color) {
        style.color = Some(c);
    }

    let Some(decls) = attr_value(&lower, "style") else {
        return style;
    };
    for decl in decls.split(';') {
        let Some((prop, val)) = decl.split_once(':') else {
            continue;
        };
        let (prop, val) = (prop.trim(), val.trim());
        match prop {
            "color" | "background" | "background-color" => {
                if let Some(c) = parse_css_color(val) {
                    style.color = Some(c);
                }
            }
            "width" => {
                if let Some(pct) = val.strip_suffix('%') {
                    if let Ok(v) = pct.trim().parse::<f64>() {
                        if v.is_finite() && v > 0.0 {
                            style.width_pct = Some(v);
                        }
                    }
                }
            }
            "height" => style.explicit_h = px(val),
            "padding" => {
                if let Some(v) = px(val) {
                    style.pad = v.min(128.0);
                }
            }
            "margin" | "margin-top" => {
                if let Some(v) = px(val) {
                    style.margin_top = v.min(256.0);
                }
            }
            "display" => {
                let v = val.trim();
                style.inline = v == "inline" || v == "inline-block" || v == "inline-flex";
            }
            _ => {}
        }
    }
    style
}

/// The value of a lowercase attribute in a lowercase attribute string.
fn attr_value<'a>(attrs_lower: &'a str, name: &str) -> Option<&'a str> {
    let mut search = 0usize;
    while let Some(pos) = attrs_lower[search..].find(name) {
        let at = search + pos;
        // Must not be part of a longer attribute name, and must be followed by
        // `=`, or it is a coincidence rather than this attribute.
        let before_ok = at == 0
            || matches!(
                attrs_lower.as_bytes()[at - 1],
                b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\''
            );
        let rest = &attrs_lower[at + name.len()..];
        let rest = rest.trim_start();
        if before_ok {
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start();
                let quote = rest.chars().next();
                return match quote {
                    Some(q @ ('"' | '\'')) => rest[1..].split(q).next(),
                    _ => rest.split_whitespace().next(),
                };
            }
        }
        search = at + name.len();
        if search >= attrs_lower.len() {
            break;
        }
    }
    None
}

/// The subset of markup the walker needs, as a flat stream.
#[derive(Debug)]
enum Token {
    /// Everything between element tags.
    Text(String),
    Open {
        tag: String,
        style: Style,
    },
    Close {
        tag: String,
    },
}

/// Stop collecting text past this much per element. The picture needs a height,
/// not the whole article, and this bounds memory on a pathological document.
const MAX_TEXT_PER_ELEMENT: usize = 4096;

/// Scan markup into a flat token stream.
///
/// Written by hand rather than with a parser dependency because the walker below
/// needs almost nothing from the markup: tag names, a couple of attributes, and
/// text lengths. Hand-rolled also means malformed markup cannot panic, which is
/// the reducer contract: a bad document is a picture, never a crash.
fn tokenize(html: &str) -> Vec<Token> {
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut tokens = Vec::with_capacity(128);
    let mut text = String::new();
    let mut i = 0usize;

    while i < n {
        if bytes[i] != b'<' {
            let start = i;
            while i < n && bytes[i] != b'<' {
                i += 1;
            }
            if text.len() < MAX_TEXT_PER_ELEMENT {
                text.push_str(&html[start..i]);
            }
            continue;
        }

        // Comments, CDATA and the doctype all end at their own terminator
        // rather than at the next `>`.
        if html[i..].starts_with("<!--") {
            push_text(&mut text, &mut tokens);
            i = match html[i..].find("-->") {
                Some(p) => i + p + 3,
                None => n,
            };
            continue;
        }
        if html[i..].starts_with("<![") {
            push_text(&mut text, &mut tokens);
            i = match html[i..].find("]]>") {
                Some(p) => i + p + 3,
                None => n,
            };
            continue;
        }
        if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
            push_text(&mut text, &mut tokens);
            i = match html[i..].find('>') {
                Some(p) => i + p + 1,
                None => n,
            };
            continue;
        }

        // A tag runs to the next `>` that is not inside a quoted attribute.
        let mut j = i + 1;
        while j < n {
            match bytes[j] {
                b'>' => break,
                q @ (b'"' | b'\'') => {
                    j += 1;
                    while j < n && bytes[j] != q {
                        j += 1;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let end = j.min(n);
        let inner = html[(i + 1).min(n)..end].trim();
        push_text(&mut text, &mut tokens);
        i = (end + 1).min(n);

        let (closing, rest) = match inner.strip_prefix('/') {
            Some(r) => (true, r.trim_start()),
            None => (false, inner),
        };
        let name_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let attrs = &rest[name_end..];

        if closing {
            tokens.push(Token::Close { tag: name });
            continue;
        }

        // `<br/>`, `<img/>` and friends: an open tag that never closes.
        let self_closing = attrs.trim_end().ends_with('/');
        tokens.push(Token::Open {
            tag: name.clone(),
            style: parse_style(attrs),
        });
        if self_closing {
            tokens.push(Token::Close { tag: name });
        }
    }

    push_text(&mut text, &mut tokens);
    tokens
}

/// Emit accumulated text as a token, dropping whitespace-only runs. Text that is
/// only a line break between two tags is layout noise, not content.
fn push_text(text: &mut String, tokens: &mut Vec<Token>) {
    if text.trim().is_empty() {
        text.clear();
        return;
    }
    tokens.push(Token::Text(std::mem::take(text)));
}

/// One open element during the walk.
///
/// `flow_y` is absolute (already offset by the parent's content origin) so that
/// a sibling can be placed without walking back up the stack.
#[derive(Clone, Debug)]
struct Frame {
    tag: String,
    /// Left edge of this element's box.
    x: f64,
    /// Top of this element's box.
    y: f64,
    w: f64,
    /// Height assumed when the element opened, used to advance the parent's
    /// flow. The real height is reconcilled on close.
    h_est: f64,
    /// Final height, set on close.
    h: f64,
    /// Width available to children inside the padding.
    content_w: f64,
    /// Where this element's content starts.
    content_y0: f64,
    /// Where the next block child goes.
    flow_y: f64,
    /// How far along the current inline row we are.
    row_x: f64,
    /// Tallest element in the current inline row.
    row_h: f64,
    margin_top: f64,
    /// Whether this element shares its parent's row, so closing it advances
    /// that row instead of starting a new one. Set from `is_row_item`, not from
    /// `is_block`: a block with an explicit width sits in a row too.
    was_inline: bool,
    /// Elements with no visual presence, which take no space and paint nothing.
    invisible: bool,
    /// Index into the painted box list, or `usize::MAX` when nothing was painted.
    box_idx: usize,
    /// Characters of direct text seen inside this element.
    text_len: usize,
}

#[allow(clippy::too_many_arguments)]
fn new_frame(
    tag: &str,
    x: f64,
    y: f64,
    w: f64,
    h_est: f64,
    content_w: f64,
    was_inline: bool,
    invisible: bool,
    margin_top: f64,
    box_idx: usize,
) -> Frame {
    Frame {
        tag: tag.to_string(),
        x,
        y,
        w,
        h_est,
        h: h_est,
        content_w,
        content_y0: y,
        flow_y: y,
        row_x: 0.0,
        row_h: 0.0,
        margin_top,
        was_inline,
        invisible,
        box_idx,
        text_len: 0,
    }
}

/// The height a tag takes when it has no children, or the height its own text
/// needs when it has no block children.
fn intrinsic_h(tag: &str, text_len: usize, content_w: f64) -> f64 {
    const LINE: f64 = 14.0;
    match tag {
        "h1" => 40.0,
        "h2" => 32.0,
        "h3" => 26.0,
        "h4" | "h5" | "h6" => 20.0,
        "img" | "video" | "canvas" | "iframe" | "svg" | "picture" => 140.0,
        "input" | "button" | "select" | "textarea" => 24.0,
        "hr" => 2.0,
        "br" => 12.0,
        "li" => 16.0,
        "td" | "th" => 20.0,
        "pre" => 48.0,
        "code" => 16.0,
        "blockquote" => 40.0,
        "table" => 60.0,
        // Pure containers get their height from their children; the floor here
        // only stops a whitespace-only element from vanishing entirely.
        "html" | "body" | "head" => 0.0,
        _ => {
            if text_len == 0 {
                8.0
            } else {
                let per_line = (content_w / 7.0).max(1.0);
                let lines = (text_len as f64 / per_line).ceil().max(1.0);
                (lines * LINE).min(1200.0)
            }
        }
    }
}

/// Darken a color slightly with nesting depth, so a box inside a box is still
/// distinguishable from its parent.
fn shade(c: (u8, u8, u8), depth: usize) -> (u8, u8, u8) {
    let f = 1.0 - (depth.min(6) as f64) * 0.06;
    (
        (c.0 as f64 * f).round() as u8,
        (c.1 as f64 * f).round() as u8,
        (c.2 as f64 * f).round() as u8,
    )
}

/// Lay a token stream out into paints.
///
/// The walk keeps a stack of open elements. A block child advances its parent's
/// `flow_y`; an inline child advances the parent's current row instead. Unknown
/// heights are estimated when an element opens and reconciled when it closes,
/// which is why every open pushes a frame rather than painting a final box.
fn layout(tokens: &[Token]) -> Vec<Box2d> {
    let mut boxes: Vec<Box2d> = Vec::new();
    let mut order: u32 = 0;
    let mut stack: Vec<Frame> = Vec::new();
    let mut root_text: usize = 0;

    for token in tokens {
        match token {
            Token::Text(t) => {
                let len = t.chars().count();
                match stack.last_mut() {
                    Some(top) => top.text_len += len,
                    None => root_text += len,
                }
            }

            Token::Open { tag, style } => {
                // Only containers with no color of their own get depth-shaded.
                // An explicit color, or a tag whose whole point is its color,
                // must paint exactly what was asked for.
                let tinted = match style
                    .color
                    .or_else(|| (tag_color(tag) != DEFAULT_TAG_COLOR).then(|| tag_color(tag)))
                {
                    Some(c) => c,
                    None => shade(DEFAULT_TAG_COLOR, stack.len()),
                };
                // Invisibility is inherited: a `<title>` inside `<head>` is as
                // unpaintable as the head itself, and a hidden subtree must not
                // contribute height to its parent.
                let invisible = is_invisible(tag) || stack.last().is_some_and(|f| f.invisible);

                let (x, y, w, h) = match stack.last_mut() {
                    Some(parent) => {
                        let w = match style.width_pct {
                            // A `%` width is a fraction of the parent's content box.
                            Some(p) => (p / 100.0 * parent.content_w).clamp(1.0, parent.content_w),
                            None => parent.content_w,
                        };
                        let h = style.explicit_h.unwrap_or_else(|| intrinsic_h(tag, 0, w));

                        if invisible {
                            // Takes no space and paints nothing, so it must not
                            // move the parent's flow.
                            (parent.flow_y, parent.flow_y, w, h)
                        } else if is_row_item(tag, style) {
                            // A block with an explicit width shares its row with
                            // its siblings, wrapping only when the row is full.
                            let wid = w.clamp(1.0, parent.content_w);
                            if parent.row_x > 0.0 && parent.row_x + wid > parent.content_w {
                                parent.flow_y += parent.row_h;
                                parent.row_x = 0.0;
                                parent.row_h = 0.0;
                            }
                            let x = parent.x + (parent.w - parent.content_w) * 0.5 + parent.row_x;
                            let y = parent.flow_y;
                            parent.row_x += wid;
                            parent.row_h = parent.row_h.max(h);
                            (x, y, wid, h)
                        } else if is_block(tag) {
                            // Anything sharing the row above it has finished.
                            parent.flow_y += parent.row_h;
                            parent.row_x = 0.0;
                            parent.row_h = 0.0;
                            let x = parent.x + (parent.w - parent.content_w) * 0.5;
                            let y = parent.flow_y + style.margin_top;
                            parent.flow_y = y + h;
                            (x, y, w, h)
                        } else {
                            // Inline: advance along the current row.
                            let x = parent.x + (parent.w - parent.content_w) * 0.5 + parent.row_x;
                            let y = parent.flow_y;
                            let wid = w.min(parent.content_w).max(24.0);
                            parent.row_x += wid;
                            parent.row_h = parent.row_h.max(h);
                            (x, y, wid, h)
                        }
                    }
                    // The document element itself has no parent to sit in.
                    None => (
                        0.0,
                        0.0,
                        style
                            .width_pct
                            .map_or(VIEWPORT_W, |p| p / 100.0 * VIEWPORT_W),
                        style.explicit_h.unwrap_or(VIEWPORT_H),
                    ),
                };

                // The document element is the canvas, not content. Painting it
                // would give every page a full-viewport background box that
                // masks whether anything visible exists at all.
                let document_element = stack.is_empty() && tag == "html";
                let box_idx = if invisible || document_element || boxes.len() >= MAX_BOXES {
                    usize::MAX
                } else {
                    boxes.push(Box2d {
                        x,
                        y,
                        w,
                        h,
                        color: tinted,
                        order,
                    });
                    order += 1;
                    boxes.len() - 1
                };

                stack.push(new_frame(
                    tag,
                    x,
                    y,
                    w,
                    h,
                    (w - style.pad * 2.0).max(1.0),
                    is_row_item(tag, style),
                    invisible,
                    style.margin_top,
                    box_idx,
                ));
                if stack.len() == 1 {
                    stack[0].text_len += root_text;
                    root_text = 0;
                }
            }

            Token::Close { tag } => {
                // A stray close tag with no matching open is ignored rather than
                // treated as an error: real documents are full of them.
                let Some(pos) = stack.iter().rposition(|f| &f.tag == tag) else {
                    continue;
                };
                close_down_to(&mut boxes, &mut stack, pos);
            }
        }
    }

    // Close anything the document left open, innermost first.
    while stack.len() > 1 {
        let pos = stack.len() - 2;
        close_down_to(&mut boxes, &mut stack, pos);
    }
    if let Some(f) = stack.pop() {
        finalize(&mut boxes, &f);
    }

    boxes
}

/// Close frames until `pos` is the top of the stack.
fn close_down_to(boxes: &mut [Box2d], stack: &mut Vec<Frame>, pos: usize) {
    loop {
        let target = stack.len() - 1 == pos;
        let f = stack.pop().expect("index came from this stack");
        let (h, bottom) = finalize(boxes, &f);
        if let Some(parent) = stack.last_mut() {
            if f.invisible {
                // Took no space, so there is nothing to propagate.
            } else if f.was_inline {
                parent.row_h = parent.row_h.max(h);
            } else {
                let b = bottom + f.margin_top;
                if b > parent.flow_y {
                    parent.flow_y = b;
                }
                parent.row_x = 0.0;
                parent.row_h = 0.0;
            }
        }
        if target {
            break;
        }
    }
}

/// Fold a closing frame's real height into its painted box, and report
/// `(height, bottom_edge)` so the caller can propagate it to the parent.
fn finalize(boxes: &mut [Box2d], f: &Frame) -> (f64, f64) {
    let natural = (f.flow_y - f.content_y0).max(f.h_est);
    let h = f.h.max(natural);
    if !f.invisible && f.box_idx != usize::MAX && f.box_idx < boxes.len() {
        boxes[f.box_idx].h = h;
    }
    (h, f.y + h)
}

/// The scaled extent of everything painted. The reducer uses this to map the
/// viewport onto the cell grid.
pub fn content_extent(boxes: &[Box2d]) -> (f64, f64) {
    let mut w = 0.0f64;
    let mut h = 0.0f64;
    for b in boxes {
        w = w.max(b.x + b.w);
        h = h.max(b.y + b.h);
    }
    (w.max(1.0), h.max(1.0))
}

/// Blend `src` over `dst` with coverage `a` in 0..=1.
fn blend(dst: (u8, u8, u8), src: (u8, u8, u8), a: f64) -> (u8, u8, u8) {
    let a = a.clamp(0.0, 1.0);
    let mix = |d: u8, s: u8| (d as f64 + (s as f64 - d as f64) * a).round() as u8;
    (mix(dst.0, src.0), mix(dst.1, src.1), mix(dst.2, src.2))
}

/// Paint `boxes` into a `side` x `side` picture.
///
/// The page is scaled to fit the grid with its aspect ratio preserved, so a
/// page taller than it is wide is letterboxed rather than squashed. A box
/// thinner than one cell still paints: bounds are widened to include one cell,
/// so a page built out of small inline elements is visible instead of being
/// dropped by a strict "one box, one cell" rule.
///
/// `view` is the region of the scaled picture to show, as `(x0, y0, w, h)` in
/// cells. The identity view is `(0, 0, n, n)`.
pub fn rasterize_view(boxes: &[Box2d], side: u32, view: (f64, f64, f64, f64)) -> Vec<(u8, u8, u8)> {
    let side = side.max(1);
    let n = side as usize;
    let mut cell = vec![(0u8, 0u8, 0u8); n * n];
    if boxes.is_empty() {
        return cell;
    }

    let (cx, cy) = content_extent(boxes);
    let scale = (n as f64 / cx).min(n as f64 / cy);
    let off_x = (n as f64 - cx * scale) * 0.5;
    let off_y = (n as f64 - cy * scale) * 0.5;

    let (vx, vy, vw, vh) = view;
    let vw = if vw.is_finite() && vw > 0.0 {
        vw
    } else {
        n as f64
    };
    let vh = if vh.is_finite() && vh > 0.0 {
        vh
    } else {
        n as f64
    };
    // Cell-space to output-space, so a zoomed view fills the whole grid.
    let (sx, sy) = (n as f64 / vw, n as f64 / vh);
    let map_x = |x: f64| (x - vx) * sx;
    let map_y = |y: f64| (y - vy) * sy;

    for b in boxes {
        if b.w <= 0.0 || b.h <= 0.0 {
            continue;
        }
        let x0 = map_x(b.x * scale + off_x);
        let y0 = map_y(b.y * scale + off_y);
        let x1 = x0 + b.w * scale * sx;
        let y1 = y0 + b.h * scale * sy;

        let gx0 = x0.floor().max(0.0) as usize;
        let gy0 = y0.floor().max(0.0) as usize;
        let gx1 = (x1.ceil().max(0.0) as usize).min(n);
        let gy1 = (y1.ceil().max(0.0) as usize).min(n);

        // A cell is 1 x 1 in output space, so the overlap area is the
        // fraction of the cell the box covers. Dividing by the box's own
        // area instead would give a fully covered cell a coverage of
        // 1 / box_area, painting big boxes almost transparent.
        let gx0 = gx0.min(n - 1);
        let gy0 = gy0.min(n - 1);
        let gx1 = gx1.max(gx0 + 1).min(n);
        let gy1 = gy1.max(gy0 + 1).min(n);

        for gy in gy0..gy1 {
            for gx in gx0..gx1 {
                // Real 2-D overlap between this cell and the box.
                let ox0 = (gx as f64).max(x0);
                let oy0 = (gy as f64).max(y0);
                let ox1 = ((gx + 1) as f64).min(x1);
                let oy1 = ((gy + 1) as f64).min(y1);
                if ox1 <= ox0 || oy1 <= oy0 {
                    continue;
                }
                let coverage = ((ox1 - ox0) * (oy1 - oy0)).clamp(0.0, 1.0);
                let idx = gy * n + gx;
                // Later boxes paint over earlier ones, weighted by coverage.
                cell[idx] = blend(cell[idx], b.color, coverage);
            }
        }
    }
    cell
}

/// The website tier: parse markup, lay it out, paint the boxes.
#[derive(Clone, Debug, Default)]
pub struct WebsiteReducer {
    pub crop: lab_core::CropScale,
}

impl WebsiteReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for WebsiteReducer {
    fn tier(&self) -> Tier {
        Tier::Website
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Website));
        }
        if side == 0 {
            return Err(ReduceError::Medium(lab_core::MediumError::Empty));
        }

        // Malformed UTF-8 must not be a hard failure: markup in the wild is
        // frequently not valid UTF-8, and replacing is what a browser does.
        let html = String::from_utf8_lossy(input);
        let tokens = tokenize(&html);

        let boxes = layout(&tokens);
        if boxes.is_empty() {
            // Well-formed markup whose elements are all invisible has nothing
            // to draw, and Empty says that better than a black grid.
            // A malformed fragment (a lone comment, stray close tags) is not
            // judged: it renders as a blank picture rather than an error.
            let has_content = tokens
                .iter()
                .any(|t| matches!(t, Token::Open { .. } | Token::Text(_)));
            if has_content {
                return Err(ReduceError::Empty(Tier::Website));
            }
        }

        // The crop widget windows the page the same way it windows an image:
        // `scale` zooms in, `x` and `y` pan. Distances are in cells, matching
        // the identity view that `rasterize` uses.
        let (view_x, _, view_w) = self.crop.columns(side as usize);
        let (view_y, _, _) = lab_core::CropScale {
            x: self.crop.y,
            y: 0.0,
            scale: self.crop.scale,
        }
        .columns(side as usize);
        let view = (view_x as f64, view_y as f64, view_w as f64, view_w as f64);

        let cells = rasterize_view(&boxes, side, view)
            .into_iter()
            .map(|(r, g, b)| Pixel::rgb(r, g, b))
            .collect();

        Medium::new(side, cells).map_err(ReduceError::from)
    }
}
