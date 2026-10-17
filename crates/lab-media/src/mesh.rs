//! Mesh reduction: a model becomes a shaded orthographic view.
//!
//! The viewer is fixed rather than interactive, so the same file always reduces
//! to the same picture. The model is rotated a little off-axis, projected onto
//! the screen plane and drawn with a z-buffer, with a single directional light
//! giving each face its shade. PLY goes in as a point cloud when it carries only
//! vertices, which is what a scan usually is.
//!
//! Parsing is written out rather than pulled in as a dependency: OBJ, STL and
//! ASCII PLY are simple, and the tier only needs positions and triangles.

use image::{Rgb, RgbImage};
use lab_core::medium::Medium;
use lab_core::reducer::{ReduceError, Reducer, Tier};

use crate::finish_canvas;

/// Screen size before the engine downscales it to the cell grid.
const CANVAS: u32 = 512;

/// Fraction of the canvas the model is allowed to fill, leaving a border.
const MARGIN: f32 = 0.9;

const MAX_VERTS: usize = 500_000;
const MAX_FACES: usize = 500_000;

struct Mesh {
    verts: Vec<[f32; 3]>,
    faces: Vec<[u32; 3]>,
}

/// The mesh tier: parse a model, render a fixed shaded view.
#[derive(Clone, Debug, Default)]
pub struct MeshReducer {
    pub crop: lab_core::CropScale,
}

impl MeshReducer {
    pub fn new(crop: lab_core::CropScale) -> Self {
        Self { crop }
    }
}

impl Reducer for MeshReducer {
    fn tier(&self) -> Tier {
        Tier::Mesh
    }

    fn reduce(&self, input: &[u8], side: u32) -> Result<Medium, ReduceError> {
        if input.is_empty() {
            return Err(ReduceError::Empty(Tier::Mesh));
        }
        let mesh = parse(input).map_err(ReduceError::Decode)?;
        if mesh.verts.is_empty() {
            return Err(ReduceError::Empty(Tier::Mesh));
        }
        let img = render(&mesh);
        finish_canvas(Tier::Mesh, img, &self.crop, side)
    }
}

fn parse(bytes: &[u8]) -> Result<Mesh, String> {
    if bytes.starts_with(b"ply") {
        return parse_ply(bytes);
    }
    if bytes.starts_with(b"solid") {
        let head = &bytes[..bytes.len().min(1024)];
        if find_subslice(head, b"facet").is_some() {
            return parse_stl_ascii(&String::from_utf8_lossy(bytes));
        }
    }
    if is_binary_stl(bytes) {
        return parse_stl_binary(bytes);
    }
    parse_obj(&String::from_utf8_lossy(bytes))
}

fn is_binary_stl(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let n = le_u32(bytes, 80) as usize;
    n > 0 && 84usize.checked_add(n * 50) == Some(bytes.len())
}

fn parse_obj(text: &str) -> Result<Mesh, String> {
    let mut verts = Vec::new();
    let mut faces = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                if let (Some(x), Some(y), Some(z)) =
                    (f32p(it.next()), f32p(it.next()), f32p(it.next()))
                {
                    if verts.len() < MAX_VERTS {
                        verts.push([x, y, z]);
                    }
                }
            }
            Some("f") => {
                let mut idx = Vec::new();
                for token in it {
                    let raw = token.split('/').next().unwrap_or("");
                    if let Ok(i) = raw.parse::<i64>() {
                        let n = verts.len() as i64;
                        let i = if i < 0 { n + i } else { i - 1 };
                        if (0..n).contains(&i) {
                            idx.push(i as u32);
                        }
                    }
                }
                for k in 1..idx.len().saturating_sub(1) {
                    if faces.len() < MAX_FACES {
                        faces.push([idx[0], idx[k], idx[k + 1]]);
                    }
                }
            }
            _ => {}
        }
    }
    if verts.is_empty() {
        return Err("OBJ has no vertices".into());
    }
    Ok(Mesh { verts, faces })
}

fn parse_stl_ascii(text: &str) -> Result<Mesh, String> {
    let mut verts = Vec::new();
    let mut faces = Vec::new();
    let mut pending: Vec<u32> = Vec::new();
    let mut it = text.split_whitespace();
    while let Some(token) = it.next() {
        if token != "vertex" {
            continue;
        }
        let (x, y, z) = (f32p(it.next()), f32p(it.next()), f32p(it.next()));
        if let (Some(x), Some(y), Some(z)) = (x, y, z) {
            if verts.len() < MAX_VERTS {
                verts.push([x, y, z]);
                pending.push(verts.len() as u32 - 1);
                if pending.len() == 3 {
                    faces.push([pending[0], pending[1], pending[2]]);
                    pending.clear();
                }
            }
        }
    }
    if verts.is_empty() {
        return Err("STL has no vertices".into());
    }
    Ok(Mesh { verts, faces })
}

fn parse_stl_binary(bytes: &[u8]) -> Result<Mesh, String> {
    let n = le_u32(bytes, 80) as usize;
    let mut verts = Vec::new();
    let mut faces = Vec::new();
    for i in 0..n {
        let base = 84 + i * 50;
        if base + 50 > bytes.len() || verts.len() + 3 > MAX_VERTS {
            break;
        }
        let push = |x: f32, y: f32, z: f32, verts: &mut Vec<[f32; 3]>| {
            verts.push([x, y, z]);
            verts.len() as u32 - 1
        };
        let a = push(
            le_f32(bytes, base + 12),
            le_f32(bytes, base + 16),
            le_f32(bytes, base + 20),
            &mut verts,
        );
        let b = push(
            le_f32(bytes, base + 24),
            le_f32(bytes, base + 28),
            le_f32(bytes, base + 32),
            &mut verts,
        );
        let c = push(
            le_f32(bytes, base + 36),
            le_f32(bytes, base + 40),
            le_f32(bytes, base + 44),
            &mut verts,
        );
        faces.push([a, b, c]);
    }
    if verts.is_empty() {
        return Err("binary STL has no triangles".into());
    }
    Ok(Mesh { verts, faces })
}

fn parse_ply(bytes: &[u8]) -> Result<Mesh, String> {
    let header_end = find_end_header(bytes).ok_or("PLY header has no end_header line")?;
    let header = String::from_utf8_lossy(&bytes[..header_end]);

    let mut binary = false;
    let mut big_endian = false;
    let mut n_vert = 0usize;
    let mut n_face = 0usize;
    let mut props: Vec<(String, usize)> = Vec::new();
    let mut in_vertex = false;

    for line in header.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("format") => {
                let f = it.next().unwrap_or("");
                binary = f.starts_with("binary");
                big_endian = f.contains("big_endian");
            }
            Some("element") => {
                let name = it.next().unwrap_or("");
                let count = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                in_vertex = name == "vertex";
                if in_vertex {
                    n_vert = count;
                    props.clear();
                } else if name == "face" {
                    n_face = count;
                }
            }
            Some("property") if in_vertex => {
                let ty = it.next().unwrap_or("");
                let name = it.next().unwrap_or("");
                if let Some(size) = type_size(ty) {
                    props.push((name.to_string(), size));
                }
            }
            _ => {}
        }
    }

    if binary && big_endian {
        return Err("big-endian binary PLY is not supported".into());
    }

    let axis = |name: &str| props.iter().position(|(n, _)| n == name);
    let (xi, yi, zi) = (axis("x"), axis("y"), axis("z"));

    let mut verts = Vec::new();
    let mut faces = Vec::new();

    if binary {
        let stride: usize = props.iter().map(|(_, s)| *s).sum();
        let offset = |target: Option<usize>| -> Option<usize> {
            let target = target?;
            props
                .iter()
                .take(target)
                .map(|(_, s)| *s)
                .sum::<usize>()
                .into()
        };
        // Only 32-bit float positions are read; other widths are rejected here
        // rather than misread, because a wrong stride would scramble the model.
        let (ox, oy, oz) = (offset(xi), offset(yi), offset(zi));
        let float_ok = |i: Option<usize>| i.map(|i| props[i].1 == 4).unwrap_or(false);
        if !(float_ok(xi) && float_ok(yi) && float_ok(zi)) || stride == 0 {
            return Err("binary PLY positions are not float32".into());
        }
        let (ox, oy, oz) = (ox.unwrap(), oy.unwrap(), oz.unwrap());
        let mut p = header_end;
        while p + stride <= bytes.len() && verts.len() < MAX_VERTS && verts.len() < n_vert {
            verts.push([
                le_f32(bytes, p + ox),
                le_f32(bytes, p + oy),
                le_f32(bytes, p + oz),
            ]);
            p += stride;
        }
    } else {
        let body = String::from_utf8_lossy(&bytes[header_end..]);
        let mut lines = body.lines().filter(|l| !l.trim().is_empty());
        for _ in 0..n_vert {
            let Some(line) = lines.next() else { break };
            if verts.len() >= MAX_VERTS {
                break;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            let get = |i: Option<usize>| i.and_then(|i| parts.get(i)).and_then(|s| s.parse().ok());
            if let (Some(x), Some(y), Some(z)) = (get(xi), get(yi), get(zi)) {
                verts.push([x, y, z]);
            }
        }
        for _ in 0..n_face {
            let Some(line) = lines.next() else { break };
            let mut it = line.split_whitespace();
            let count: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let idx: Vec<u32> = it.take(count).filter_map(|s| s.parse().ok()).collect();
            for k in 1..idx.len().saturating_sub(1) {
                if faces.len() < MAX_FACES {
                    faces.push([idx[0], idx[k], idx[k + 1]]);
                }
            }
        }
    }

    if verts.is_empty() {
        return Err("PLY has no vertices".into());
    }
    Ok(Mesh { verts, faces })
}

fn render(mesh: &Mesh) -> RgbImage {
    let yaw = 0.6f32;
    let pitch = -0.35f32;
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let (cp, sp) = (pitch.cos(), pitch.sin());

    let center = {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for v in &mesh.verts {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ]
    };

    let tv: Vec<[f32; 3]> = mesh
        .verts
        .iter()
        .map(|v| {
            let x0 = v[0] - center[0];
            let y0 = v[1] - center[1];
            let z0 = v[2] - center[2];
            let x = x0 * cy + z0 * sy;
            let z = -x0 * sy + z0 * cy;
            [x, y0 * cp - z * sp, y0 * sp + z * cp]
        })
        .collect();

    let (mut x0, mut x1, mut y0, mut y1) = (
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    );
    for p in &tv {
        x0 = x0.min(p[0]);
        x1 = x1.max(p[0]);
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    let span = (x1 - x0).max(y1 - y0).max(1e-6);
    let scale = CANVAS as f32 * MARGIN / span;
    let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let proj = |p: [f32; 3]| -> (f32, f32, f32) {
        (
            CANVAS as f32 * 0.5 + (p[0] - mx) * scale,
            CANVAS as f32 * 0.5 - (p[1] - my) * scale,
            p[2],
        )
    };

    let mut img = RgbImage::from_pixel(CANVAS, CANVAS, Rgb([12, 12, 14]));
    let mut zbuf = vec![f32::INFINITY; (CANVAS * CANVAS) as usize];

    if mesh.faces.is_empty() {
        let zmin = tv.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        let zmax = tv.iter().map(|p| p[2]).fold(f32::NEG_INFINITY, f32::max);
        let depth = (zmax - zmin).max(1e-6);
        for p in &tv {
            let (sx, sy2, z) = proj(*p);
            let (xi, yi) = (sx as i32, sy2 as i32);
            if xi < 0 || yi < 0 || xi >= CANVAS as i32 || yi >= CANVAS as i32 {
                continue;
            }
            let idx = (yi as u32 * CANVAS + xi as u32) as usize;
            if z < zbuf[idx] {
                zbuf[idx] = z;
                let t = 1.0 - (z - zmin) / depth;
                let b = (90.0 + 150.0 * t) as u8;
                img.put_pixel(xi as u32, yi as u32, Rgb([b, b, b.saturating_add(12)]));
            }
        }
    } else {
        let light = normalize([0.4, 0.6, 0.7]);
        for f in &mesh.faces {
            if f.iter().any(|&i| i as usize >= tv.len()) {
                continue;
            }
            let (a, b, c) = (tv[f[0] as usize], tv[f[1] as usize], tv[f[2] as usize]);
            let n = normalize(cross(sub(b, a), sub(c, a)));
            let lite = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).abs();
            let shade = (0.15 + 0.9 * lite).clamp(0.0, 1.0);
            let color = Rgb([
                (shade * 200.0) as u8,
                (shade * 205.0) as u8,
                (shade * 220.0) as u8,
            ]);
            let (ax, ay, az) = proj(a);
            let (bx, by, bz) = proj(b);
            let (cx, cyy, cz) = proj(c);
            rasterize(
                &mut img,
                &mut zbuf,
                [ax, ay, az],
                [bx, by, bz],
                [cx, cyy, cz],
                color,
            );
        }
    }
    img
}

#[allow(clippy::too_many_arguments)]
fn rasterize(
    img: &mut RgbImage,
    zbuf: &mut [f32],
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    color: Rgb<u8>,
) {
    let area = edge(a, b, c);
    if area.abs() < 1e-6 {
        return;
    }
    let minx = a[0].min(b[0]).min(c[0]).floor().max(0.0) as i32;
    let maxx = a[0].max(b[0]).max(c[0]).ceil().min(CANVAS as f32 - 1.0) as i32;
    let miny = a[1].min(b[1]).min(c[1]).floor().max(0.0) as i32;
    let maxy = a[1].max(b[1]).max(c[1]).ceil().min(CANVAS as f32 - 1.0) as i32;
    for y in miny..=maxy {
        for x in minx..=maxx {
            let p = [x as f32 + 0.5, y as f32 + 0.5, 0.0];
            let (w0, w1, w2) = (edge(b, c, p), edge(c, a, p), edge(a, b, p));
            if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) {
                let z = (w0 * a[2] + w1 * b[2] + w2 * c[2]) / area;
                let idx = (y as u32 * CANVAS + x as u32) as usize;
                if z < zbuf[idx] {
                    zbuf[idx] = z;
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}

fn edge(a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> f32 {
    (p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / len, v[1] / len, v[2] / len]
}

fn f32p(s: Option<&str>) -> Option<f32> {
    s?.parse().ok()
}

fn type_size(t: &str) -> Option<usize> {
    Some(match t {
        "char" | "int8" | "uint8" | "uchar" => 1,
        "short" | "int16" | "ushort" | "uint16" => 2,
        "int" | "int32" | "uint" | "uint32" | "float" | "float32" => 4,
        "double" | "float64" => 8,
        _ => return None,
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_end_header(bytes: &[u8]) -> Option<usize> {
    let idx = find_subslice(bytes, b"end_header")?;
    let mut p = idx + b"end_header".len();
    while p < bytes.len() && bytes[p] != b'\n' {
        p += 1;
    }
    Some((p + 1).min(bytes.len()))
}

fn le_u32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn le_f32(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
