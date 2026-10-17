//! The engine, reachable from a web page.
//!
//! This crate is a thin ABI over `lab-core` and `lab-media`, not a second
//! implementation: the page uploads a file, this reduces it to the same N x N
//! grid the CLI would, and `labronify_solve` runs the same solver. The one
//! addition is the trace: every generation calls back into the page so it can
//! draw the grid as it converges, which is the whole reason to run in-browser.
//!
//! There is no wasm-bindgen here on purpose. The surface is a handful of
//! `extern "C"` functions over linear memory, which keeps the module small and
//! the page free of a toolchain. Callers free returned buffers with
//! [`labronify_free`].
//!
//! Everything is `#[no_mangle]` `extern "C"` and pointer-based, so the same
//! source builds natively (unused) and for `wasm32-unknown-unknown` (shipped).

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use lab_core::{Medium, Pixel, Settings};
use lab_media::reduce_file;

// The page hands each generation's state to this import: `gen` is the 1-based
// generation number, `rgb` is `side*side*3` bytes in target order, and
// `assign` is `side*side` u32s (source cell per target position) so the page can
// morph cells toward it. `wasm_import_module` is what makes rustc emit a wasm
// import instead of an undefined symbol.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
extern "C" {
    fn labronify_frame(gen: u32, rgb: *const u8, rgb_len: usize, assign: *const u32, n: usize);
}

// Native builds never call it; the stub keeps native builds linking.
#[cfg(not(target_arch = "wasm32"))]
unsafe extern "C" fn labronify_frame(
    _gen: u32,
    _rgb: *const u8,
    _rgb_len: usize,
    _a: *const u32,
    _n: usize,
) {
}

/// Reserve `len` bytes and return the pointer, so the page can copy a file in.
/// A zero-length request still returns a valid, freeable pointer.
#[no_mangle]
pub extern "C" fn labronify_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Free a buffer previously returned by any exported function, given the same
/// capacity it was created with.
#[no_mangle]
pub extern "C" fn labronify_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
    }
}

/// Reduce a file's bytes to the engine's grid. `ext` selects the tier and
/// `side` the grid length; `crop_x`/`crop_y`/`crop_scale` are the same source
/// crop the CLI takes. Returns a pointer to `side*side*3` RGB bytes, or null if
/// the tier is unknown or the bytes do not decode.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn labronify_reduce(
    data: *const u8,
    len: usize,
    ext: *const u8,
    ext_len: usize,
    side: u32,
    crop_x: f32,
    crop_y: f32,
    crop_scale: f32,
) -> *mut u8 {
    if data.is_null() || side == 0 {
        return std::ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    let ext = unsafe { std::slice::from_raw_parts(ext, ext_len) };
    let ext = std::str::from_utf8(ext).unwrap_or("");
    let crop = lab_core::CropScale {
        x: crop_x,
        y: crop_y,
        scale: if crop_scale > 0.0 { crop_scale } else { 1.0 },
    };

    match reduce_file(bytes, ext, side, crop) {
        Ok(medium) => {
            let mut out = Vec::with_capacity(medium.cell_count() * 3);
            for (r, g, b) in medium.tuples() {
                out.push(r);
                out.push(g);
                out.push(b);
            }
            into_leak(out)
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Solve the grid against the LeBron target: call [`labronify_reduce`]'s output.
/// `grid` is `side*side*3` RGB bytes and `proximity` the raw slider value (it is
/// calibrated here, exactly as the CLI does at solve time). Every generation is
/// pushed to the page through `labronify_frame`. Returns a pointer to the
/// `side*side` assignment as `u32` little-endian (`side*side*4` bytes), or null
/// on failure. Bytes, not words, so one `labronify_free` covers every buffer.
#[no_mangle]
pub extern "C" fn labronify_solve(grid: *const u8, side: u32, proximity: i32) -> *mut u8 {
    run(
        grid,
        std::ptr::null(),
        side,
        proximity,
        false,
        None,
        0,
        true,
    )
}

/// [`labronify_solve`] against a caller-supplied target grid instead of the baked
/// LeBron target. `target` is `side*side*3` RGB bytes; null means LeBron. The
/// importance mask stays the baked one, exactly as the reference keeps its
/// weights fixed while the target image changes. `optimal` picks the slow exact
/// solver instead of the genetic one.
#[no_mangle]
pub extern "C" fn labronify_solve_on(
    grid: *const u8,
    target: *const u8,
    side: u32,
    proximity: i32,
    optimal: u32,
) -> *mut u8 {
    run(grid, target, side, proximity, optimal != 0, None, 0, true)
}

/// [`labronify_solve`] starting from a previous assignment. `prev` points to
/// `side*side` `u32` little-endian (null means start from the identity). Video
/// callers hand back the previous frame's result here: near-identical frames
/// then converge in a few generations instead of a full solve.
#[no_mangle]
pub extern "C" fn labronify_solve_warm(
    grid: *const u8,
    side: u32,
    proximity: i32,
    prev: *const u8,
    max_gens: u32,
) -> *mut u8 {
    let initial = if prev.is_null() || side == 0 {
        None
    } else {
        let n = (side as usize) * (side as usize);
        let raw = unsafe { std::slice::from_raw_parts(prev, n * 4) };
        Some(
            raw.chunks_exact(4)
                .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as usize)
                .collect::<Vec<_>>(),
        )
    };
    run(
        grid,
        std::ptr::null(),
        side,
        proximity,
        false,
        initial,
        max_gens,
        false,
    )
}

/// `max_gens` caps the generations (0 = run to convergence). The stopping rule
/// gates convergence on `max_dist` decaying to near zero, which with `u32`
/// truncation takes about `side` generations no matter how good the start is. So
/// a frame-to-frame caller cannot lean on convergence alone: it warms from the
/// last frame and caps the work, and each frame adapts in a handful of
/// generations instead of paying the full `side`.
#[allow(clippy::too_many_arguments)]
fn run(
    grid: *const u8,
    target: *const u8,
    side: u32,
    proximity: i32,
    optimal: bool,
    initial: Option<Vec<usize>>,
    max_gens: u32,
    stream: bool,
) -> *mut u8 {
    if grid.is_null() || side == 0 {
        return std::ptr::null_mut();
    }
    let proximity = proximity as i64;
    let n = (side as usize) * (side as usize);
    let rgb = unsafe { std::slice::from_raw_parts(grid, n * 3) };

    let cells = rgb
        .chunks_exact(3)
        .map(|c| Pixel::rgb(c[0], c[1], c[2]))
        .collect();
    let source = match Medium::new(side, cells) {
        Ok(m) => m,
        Err(_) => return std::ptr::null_mut(),
    };

    let settings = Settings {
        sidelen: side,
        proximity_importance: proximity,
        algorithm: if optimal {
            lab_core::Algorithm::Optimal
        } else {
            lab_core::Algorithm::Genetic
        },
        ..Default::default()
    }
    .calibrated();

    let baked = match lab_core::target_and_weights(&settings) {
        Ok(tw) => tw,
        Err(_) => return std::ptr::null_mut(),
    };
    let weights = baked.1;
    let target = if target.is_null() {
        baked.0
    } else {
        let t = unsafe { std::slice::from_raw_parts(target, n * 3) };
        match Medium::new(
            side,
            t.chunks_exact(3)
                .map(|c| Pixel::rgb(c[0], c[1], c[2]))
                .collect(),
        ) {
            Ok(m) => m,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let gens = std::cell::Cell::new(0u32);
    let mut sink = |gen: u32, frame: &[u8], assign: &[u32]| {
        gens.set(gen);
        if stream {
            // The import only reads; it never keeps the slices past the call.
            unsafe {
                labronify_frame(
                    gen,
                    frame.as_ptr(),
                    frame.len(),
                    assign.as_ptr(),
                    assign.len(),
                )
            };
        }
    };
    let should_stop = || max_gens != 0 && gens.get() >= max_gens;

    let solution = lab_core::solve_from(
        &source,
        &target,
        &weights,
        settings.proximity_importance,
        settings.algorithm,
        &should_stop,
        initial.as_deref(),
        &mut sink,
    );

    let mut out = Vec::with_capacity(n * 4);
    for &s in &solution.assignment {
        out.extend_from_slice(&(s as u32).to_le_bytes());
    }
    into_leak(out)
}

/// Move a `Vec`'s buffer out to the page and return its pointer. The caller
/// frees it with [`labronify_free`] using the element count it asked for.
fn into_leak<T>(mut v: Vec<T>) -> *mut T {
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr
}
