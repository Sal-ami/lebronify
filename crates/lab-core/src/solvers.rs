//! Solvers.
//!
//! Both solvers answer the same question: which permutation of the source cells
//! minimizes total cost when laid into the target grid? `Genetic` (the default)
//! is a forced-move hill climb; `Optimal` is Kuhn-Munkres.
//!
//! The genetic solver's structure is ported from the reference implementation:
//! fixed RNG seed `12345`, `SWAPS_PER_GENERATION_PER_PIXEL` proposals per
//! generation, a starting `max_dist` of `sidelen`, and `max_dist *= 0.99`
//! (floor 2) after every generation. The stopping rule is the reference's:
//! stop once `max_dist < 4 && swaps_made < 10`.
//!
//! "Generation" is kept as the outer unit of work because it is the natural
//! granularity for progress reporting and cancellation. A long run must be
//! interruptible, so the loop also accepts a `should_stop` predicate which
//! returns the best assignment found so far rather than throwing the work away.

use crate::cost::cost;
use crate::medium::Medium;
use crate::settings::Algorithm;

/// Reference value. Not tunable: it is part of the solver's identity.
pub const SWAPS_PER_GENERATION_PER_PIXEL: usize = 128;
/// Reference seed for the genetic solver's PRNG.
pub const GENETIC_SEED: u64 = 12345;
/// `max_dist` shrinks by this factor each generation.
pub const MAX_DIST_DECAY: f32 = 0.99;
/// `max_dist` never decays below this.
pub const MAX_DIST_FLOOR: u32 = 2;
/// Convergence thresholds, from the reference stopping rule.
pub const CONVERGED_MAX_DIST: u32 = 4;
pub const CONVERGED_SWAPS: u32 = 10;

/// Why a solve stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolveOutcome {
    /// Hit the reference convergence rule.
    Converged,
    /// The caller's `should_stop` predicate fired. The returned assignment is
    /// the best one reached, not a partial or invalid state.
    Stopped,
}

#[derive(Clone, Debug)]
pub struct Solution {
    /// `assignment[target_index] = source_index`.
    pub assignment: Vec<usize>,
    /// Total cost of the returned assignment.
    pub cost: i64,
    pub generations: u32,
    pub outcome: SolveOutcome,
}

/// One placed cell: which source cell sits here, where it came from, its color,
/// and its current cost.
#[derive(Clone, Copy)]
struct Placed {
    src: u32,
    sx: u16,
    sy: u16,
    rgb: (u8, u8, u8),
    h: i64,
}

/// One generation's state, handed to a trace sink: `gen` is the 1-based
/// generation number, `rgb` is the current cell colors in target order
/// (`side*side*3` bytes), and `assignment` is `side*side` entries mapping each
/// target position to the source cell sitting there. A browser animates the
/// solve by drawing the colors; the assignment lets it move cells there.
pub type GenSink<'a> = &'a mut dyn FnMut(u32, &[u8], &[u32]);

pub fn solve(
    source: &Medium,
    target: &Medium,
    weights: &[i64],
    proximity_importance: i64,
    algorithm: Algorithm,
    should_stop: &dyn Fn() -> bool,
) -> Solution {
    solve_traced(
        source,
        target,
        weights,
        proximity_importance,
        algorithm,
        should_stop,
        &mut |_, _, _| {},
    )
}

/// [`solve`] plus a per-generation trace. The sink sees the grid after every
/// generation, so a caller can watch it converge. Tracing does not change the
/// swap sequence: it only reads the state the solver already holds.
pub fn solve_traced(
    source: &Medium,
    target: &Medium,
    weights: &[i64],
    proximity_importance: i64,
    algorithm: Algorithm,
    should_stop: &dyn Fn() -> bool,
    on_generation: GenSink,
) -> Solution {
    solve_from(
        source,
        target,
        weights,
        proximity_importance,
        algorithm,
        should_stop,
        None,
        on_generation,
    )
}

/// [`solve_traced`] starting from `initial` instead of the identity.
///
/// The solver normally assumes nothing about where cells should go. A caller
/// that already has a good answer for a nearly-identical problem (the previous
/// video frame) can hand it back as `initial` and converge in a few generations
/// rather than from scratch. `None` is the identity and behaves exactly like
/// [`solve`]. `initial` must be a permutation of `0..side*side`.
#[allow(clippy::too_many_arguments)]
pub fn solve_from(
    source: &Medium,
    target: &Medium,
    weights: &[i64],
    proximity_importance: i64,
    algorithm: Algorithm,
    should_stop: &dyn Fn() -> bool,
    initial: Option<&[usize]>,
    on_generation: GenSink,
) -> Solution {
    match algorithm {
        Algorithm::Genetic => solve_genetic(
            source,
            target,
            weights,
            proximity_importance,
            should_stop,
            initial,
            on_generation,
        ),
        Algorithm::Optimal => {
            solve_optimal(source, target, weights, proximity_importance, should_stop)
        }
    }
}
/// How often the inner proposal loop polls the cancellation predicate. Does not
/// affect the algorithm: it only reads a flag, so the swap sequence is identical
/// to a run that never polls.
const STOP_POLL_INTERVAL: usize = 1 << 16;

#[allow(clippy::too_many_arguments)]
fn solve_genetic(
    source: &Medium,
    target: &Medium,
    weights: &[i64],
    proximity_importance: i64,
    should_stop: &dyn Fn() -> bool,
    initial: Option<&[usize]>,
    on_generation: GenSink,
) -> Solution {
    let side = target.side() as usize;
    assert_eq!(source.side(), target.side(), "source/target side mismatch");
    assert_eq!(
        weights.len(),
        side * side,
        "weights must have one entry per target cell"
    );
    if let Some(init) = initial {
        assert_eq!(
            init.len(),
            side * side,
            "initial assignment has wrong length"
        );
    }

    let source_t = source.tuples();
    let target_t = target.tuples();

    // Grid coordinates for every index. The proposal loop needs a cell's x/y
    // several times per iteration, and deriving them with `/` and `%` each time
    // was the loop's costliest step; the tables are the same values.
    let col: Vec<u16> = (0..side * side).map(|i| (i % side) as u16).collect();
    let row: Vec<u16> = (0..side * side).map(|i| (i / side) as u16).collect();

    // pixels[i] is the cell currently sitting at target position i. Without an
    // `initial` that is the identity; with one, each target already holds the
    // cell the previous solve put there.
    let mut pixels: Vec<Placed> = (0..source_t.len())
        .map(|i| {
            let src = initial.map_or(i, |init| init[i]);
            let rgb = source_t[src];
            let pos = (col[src], row[src]);
            let h = cost(
                pos,
                (col[i], row[i]),
                rgb,
                target_t[i],
                weights[i],
                proximity_importance,
            );
            Placed {
                src: src as u32,
                sx: pos.0,
                sy: pos.1,
                rgb,
                h,
            }
        })
        .collect();

    let mut rng = frand::Rand::with_seed(GENETIC_SEED);
    let proposals_per_generation = SWAPS_PER_GENERATION_PER_PIXEL * pixels.len();
    let mut max_dist = target.side();
    let mut generations = 0u32;
    let mut outcome = SolveOutcome::Stopped;
    // Reused across generations: the trace sink never keeps the slices.
    let mut trace = vec![0u8; pixels.len() * 3];
    let mut assign = vec![0u32; pixels.len()];

    'generations: loop {
        let mut swaps_made = 0u32;

        for proposal in 0..proposals_per_generation {
            if proposal % STOP_POLL_INTERVAL == 0 && should_stop() {
                break 'generations;
            }

            let apos = rng.gen_range(0..pixels.len() as u32) as usize;
            let ax = col[apos];
            let ay = row[apos];

            let bx = (ax as i32 + rng.gen_range(-(max_dist as i32)..(max_dist as i32 + 1)))
                .clamp(0, side as i32 - 1) as u16;
            let by = (ay as i32 + rng.gen_range(-(max_dist as i32)..(max_dist as i32 + 1)))
                .clamp(0, side as i32 - 1) as u16;
            let bpos = by as usize * side + bx as usize;

            if apos == bpos {
                continue;
            }

            let a = pixels[apos];
            let b = pixels[bpos];

            // Cost of a's cell moved onto b's position, and vice versa.
            let a_on_b = cost(
                (a.sx, a.sy),
                (bx, by),
                a.rgb,
                target_t[bpos],
                weights[bpos],
                proximity_importance,
            );
            let b_on_a = cost(
                (b.sx, b.sy),
                (ax, ay),
                b.rgb,
                target_t[apos],
                weights[apos],
                proximity_importance,
            );

            let improvement_a = a.h - b_on_a;
            let improvement_b = b.h - a_on_b;
            if improvement_a + improvement_b > 0 {
                pixels.swap(apos, bpos);
                pixels[apos].h = b_on_a;
                pixels[bpos].h = a_on_b;
                swaps_made += 1;
            }
        }

        generations += 1;

        for (i, p) in pixels.iter().enumerate() {
            trace[i * 3] = p.rgb.0;
            trace[i * 3 + 1] = p.rgb.1;
            trace[i * 3 + 2] = p.rgb.2;
            assign[i] = p.src;
        }
        on_generation(generations, &trace, &assign);

        if max_dist < CONVERGED_MAX_DIST && swaps_made < CONVERGED_SWAPS {
            outcome = SolveOutcome::Converged;
            break;
        }
        if should_stop() {
            break;
        }

        max_dist = (max_dist as f32 * MAX_DIST_DECAY).max(MAX_DIST_FLOOR as f32) as u32;
    }

    let assignment = pixels.iter().map(|p| p.src as usize).collect::<Vec<_>>();
    let total = pixels.iter().map(|p| p.h).sum();
    Solution {
        assignment,
        cost: total,
        generations,
        outcome,
    }
}
/// Kuhn-Munkres (Hungarian) assignment, inlined from `pathfinding::kuhn_munkres`
/// exactly as the reference app inlines it, so that the solver can be polled for
/// cancellation. Weights are negated costs because the algorithm maximizes.
fn solve_optimal(
    source: &Medium,
    target: &Medium,
    weights: &[i64],
    proximity_importance: i64,
    should_stop: &dyn Fn() -> bool,
) -> Solution {
    let side = target.side() as usize;
    assert_eq!(source.side(), target.side(), "source/target side mismatch");
    assert_eq!(weights.len(), side * side);

    let n = source.cell_count();
    let source_t = source.tuples();
    let target_t = target.tuples();

    // Maximize the negative cost. `at(row, col)`: `row` is a target cell, `col`
    // is a source cell.
    let at = |row: usize, col: usize| -> i64 {
        let (x1, y1) = ((row % side) as u16, (row / side) as u16);
        let (x2, y2) = ((col % side) as u16, (col / side) as u16);
        -cost(
            (x2, y2),
            (x1, y1),
            source_t[col],
            target_t[row],
            weights[row],
            proximity_importance,
        )
    };

    let mut xy: Vec<Option<usize>> = vec![None; n];
    let mut yx: Vec<Option<usize>> = vec![None; n];
    let mut lx: Vec<i64> = (0..n)
        .map(|row| (0..n).map(|c| at(row, c)).max().unwrap())
        .collect();
    let mut ly: Vec<i64> = vec![0; n];
    let mut s = vec![false; n];
    let mut alternating: Vec<Option<usize>> = vec![None; n];
    let mut slack = vec![0i64; n];
    let mut slackx = vec![0usize; n];

    for root in 0..n {
        if should_stop() {
            return finish_optimal(
                &xy,
                source_t,
                target_t,
                weights,
                proximity_importance,
                side,
                root as u32,
                SolveOutcome::Stopped,
            );
        }

        for y in alternating.iter_mut() {
            *y = None;
        }
        for x in s.iter_mut() {
            *x = false;
        }
        s[root] = true;
        for y in 0..n {
            slack[y] = lx[root] + ly[y] - at(root, y);
        }
        for x in slackx.iter_mut() {
            *x = root;
        }

        let y_found: usize;
        loop {
            // Select the tightest edge from the frontier.
            let mut delta = i64::MAX;
            let mut x = 0usize;
            let mut y = 0usize;
            for yy in 0..n {
                if alternating[yy].is_none() && slack[yy] < delta {
                    delta = slack[yy];
                    x = slackx[yy];
                    y = yy;
                }
            }
            if delta > 0 {
                for xi in 0..n {
                    if s[xi] {
                        lx[xi] -= delta;
                    }
                }
                for yy in 0..n {
                    if alternating[yy].is_some() {
                        ly[yy] += delta;
                    } else {
                        slack[yy] -= delta;
                    }
                }
            }
            alternating[y] = Some(x);
            if yx[y].is_none() {
                y_found = y;
                break;
            }
            let x = yx[y].unwrap();
            s[x] = true;
            for yy in 0..n {
                if alternating[yy].is_none() {
                    let alternate_slack = lx[x] + ly[yy] - at(x, yy);
                    if slack[yy] > alternate_slack {
                        slack[yy] = alternate_slack;
                        slackx[yy] = x;
                    }
                }
            }
        }

        // Reverse edges along the augmenting path.
        let mut y = Some(y_found);
        while let Some(yy) = y {
            let x = alternating[yy].unwrap();
            let prec = xy[x];
            yx[yy] = Some(x);
            xy[x] = Some(yy);
            y = prec;
        }
    }

    finish_optimal(
        &xy,
        source_t,
        target_t,
        weights,
        proximity_importance,
        side,
        n as u32,
        SolveOutcome::Converged,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_optimal(
    xy: &[Option<usize>],
    source_t: Vec<(u8, u8, u8)>,
    target_t: Vec<(u8, u8, u8)>,
    weights: &[i64],
    proximity_importance: i64,
    side: usize,
    generations: u32,
    outcome: SolveOutcome,
) -> Solution {
    // Cells the algorithm never got to match fall back to their own position.
    // This only happens on early cancellation, where the result is explicitly
    // labelled `Stopped`.
    let assignment: Vec<usize> = xy
        .iter()
        .enumerate()
        .map(|(row, m)| match m {
            Some(col) => *col,
            None => row,
        })
        .collect();

    let mut total = 0i64;
    for (row, col) in assignment.iter().enumerate() {
        total += cost(
            ((*col % side) as u16, (*col / side) as u16),
            ((row % side) as u16, (row / side) as u16),
            source_t[*col],
            target_t[row],
            weights[row],
            proximity_importance,
        );
    }

    Solution {
        assignment,
        cost: total,
        generations,
        outcome,
    }
}
