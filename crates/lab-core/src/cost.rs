//! The cost model.
//!
//! Ported verbatim from obamify's `src/app/calculate/mod.rs` (`heuristic`).
//! Frozen: it defines what "looks like LeBron" means. Do not refactor, reorder
//! or "simplify" it. The `(spatial * spatial_weight).pow(2)`
//! is not a typo for `spatial * spatial_weight.pow(2)`; the squaring of the
//! already-multiplied spatial term is what makes the proximity slider behave
//! the way the reference app behaves.
//!
//! Lower is better. Callers negate it when a maximization algorithm wants it.

/// Squared spatial distance between two cell coordinates, in cells.
#[inline(always)]
pub fn spatial_term(apos: (u16, u16), bpos: (u16, u16)) -> i64 {
    (apos.0 as i64 - bpos.0 as i64).pow(2) + (apos.1 as i64 - bpos.1 as i64).pow(2)
}

/// Squared RGB distance between two cell colors.
#[inline(always)]
pub fn color_term(a: (u8, u8, u8), b: (u8, u8, u8)) -> i64 {
    (a.0 as i64 - b.0 as i64).pow(2)
        + (a.1 as i64 - b.1 as i64).pow(2)
        + (a.2 as i64 - b.2 as i64).pow(2)
}

/// Cost of putting source cell `a` at target cell `b`.
///
/// `color_weight` is the per-target-cell importance mask (the LeBron silhouette
/// at 256x256, downsampled). `spatial_weight` is `settings.proximity_importance`.
#[inline(always)]
pub fn cost(
    apos: (u16, u16),
    bpos: (u16, u16),
    a: (u8, u8, u8),
    b: (u8, u8, u8),
    color_weight: i64,
    spatial_weight: i64,
) -> i64 {
    let spatial = spatial_term(apos, bpos);
    let color = color_term(a, b);
    color * color_weight + (spatial * spatial_weight).pow(2)
}

/// The importance mask, decoded from the single-channel RGBA PNG obamify ships.
pub fn load_weights(pixels: &[(u8, u8, u8)], width: u32, height: u32) -> Vec<i64> {
    let mut weights = vec![0i64; (width * height) as usize];
    for (i, p) in pixels.iter().enumerate() {
        weights[i] = p.0 as i64;
    }
    weights
}
