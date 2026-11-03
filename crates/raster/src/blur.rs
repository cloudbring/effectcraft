//! Shared variance-exact Gaussian filtering, explicit box blurs, and directional/radial blurs.

use craft_blur::{Cancel, Edge, Error, Gaussian, LINE_BATCH, Quality, Scratch, Shape};
use rayon::prelude::*;

use crate::{Image, Px};

/// One horizontal box pass of radius `r` (window 2r+1) over each row of `src` (width `w`) into
/// `out`; `repeat` = edge pixels extend, otherwise transparent.
fn box_h_into(src: &[Px], out: &mut [Px], w: usize, r: usize, repeat: bool) {
    if r == 0 {
        out.copy_from_slice(src);
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    out.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(o, row)| {
        let get = |i: isize| -> Px {
            if i < 0 {
                if repeat { row[0] } else { [0.0; 4] }
            } else if i as usize >= w {
                if repeat { row[w - 1] } else { [0.0; 4] }
            } else {
                row[i as usize]
            }
        };
        let mut acc = [0.0f32; 4];
        for i in -(r as isize)..=(r as isize) {
            let p = get(i);
            for c in 0..4 {
                acc[c] += p[c];
            }
        }
        for x in 0..w {
            o[x] = acc.map(|v| v * norm);
            let add = get(x as isize + r as isize + 1);
            let sub = get(x as isize - r as isize);
            for c in 0..4 {
                acc[c] += add[c] - sub[c];
            }
        }
    });
}

/// One vertical box pass (same semantics as [`box_h_into`] along columns).
fn box_v_into(src: &[Px], out: &mut [Px], w: usize, h: usize, r: usize, repeat: bool) {
    if r == 0 {
        out.copy_from_slice(src);
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    let block = (4 * r).clamp(32, 256);
    let row = |i: isize| -> Option<&[Px]> {
        if i < 0 {
            repeat.then(|| &src[0..w])
        } else if i as usize >= h {
            repeat.then(|| &src[(h - 1) * w..h * w])
        } else {
            Some(&src[i as usize * w..(i as usize + 1) * w])
        }
    };
    out.par_chunks_mut(w * block).enumerate().for_each(|(bi, chunk)| {
        let ya = (bi * block) as isize;
        let mut acc = vec![[0.0f32; 4]; w];
        for i in ya - r as isize..=ya + r as isize {
            if let Some(rw) = row(i) {
                for (a, p) in acc.iter_mut().zip(rw) {
                    for c in 0..4 {
                        a[c] += p[c];
                    }
                }
            }
        }
        for (yy, o) in chunk.chunks_mut(w).enumerate() {
            let y = ya + yy as isize;
            for (o, a) in o.iter_mut().zip(&acc) {
                *o = a.map(|v| v * norm);
            }
            match (row(y + r as isize + 1), row(y - r as isize)) {
                (Some(add), Some(sub)) => {
                    for ((a, p), q) in acc.iter_mut().zip(add).zip(sub) {
                        for c in 0..4 {
                            a[c] += p[c] - q[c];
                        }
                    }
                }
                (Some(add), None) => {
                    for (a, p) in acc.iter_mut().zip(add) {
                        for c in 0..4 {
                            a[c] += p[c];
                        }
                    }
                }
                (None, Some(sub)) => {
                    for (a, q) in acc.iter_mut().zip(sub) {
                        for c in 0..4 {
                            a[c] -= q[c];
                        }
                    }
                }
                (None, None) => {}
            }
        }
    });
}

/// Run horizontal passes with radii `rx` then vertical passes with radii `ry`.
fn box_passes(img: &Image, rx: &[usize], ry: &[usize], repeat: bool) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let passes: Vec<(bool, usize)> = rx.iter().map(|&r| (false, r)).chain(ry.iter().map(|&r| (true, r))).filter(|p| p.1 > 0).collect();
    if passes.is_empty() || w == 0 || h == 0 {
        return img.clone();
    }
    let mut a = Image::new(img.width, img.height);
    let mut b = Image::new(img.width, img.height);
    for (i, &(vertical, r)) in passes.iter().enumerate() {
        let src: &[Px] = if i == 0 { &img.data } else { &a.data };
        if vertical {
            box_v_into(src, &mut b.data, w, h, r, repeat);
        } else {
            box_h_into(src, &mut b.data, w, r, repeat);
        }
        std::mem::swap(&mut a, &mut b);
    }
    a
}

/// Separable box blur with given radii, `iterations` passes.
pub fn box_blur(img: &Image, rx: usize, ry: usize, iterations: usize, repeat: bool) -> Image {
    let n = iterations.max(1);
    box_passes(img, &vec![rx; n], &vec![ry; n], repeat)
}

/// Variance-exact Gaussian with standard deviations in pixels, on premultiplied RGBA.
pub fn gaussian_blur(img: &Image, sigma_x: f64, sigma_y: f64, repeat: bool) -> Image {
    gaussian_blur_with_quality(img, sigma_x, sigma_y, repeat, Quality::Recursive)
}

/// The old Gaussian effect's sampled/box crossover, retained for project compatibility and GPU
/// dispatch. The default CPU filter owns its own sampled/recursive crossover in craft-blur.
const EXACT_SIGMA_BELOW: f64 = 3.0;

/// Whether a legacy Gaussian uses a sampled kernel rather than quantized boxes.
pub fn gaussian_is_exact(sigma_x: f64, sigma_y: f64) -> bool {
    sigma_x.max(sigma_y) < EXACT_SIGMA_BELOW
}

/// Explicit quality selection, including the legacy three-box approximation. Render functions
/// cannot return errors; failures are logged and preserve the unmodified input. Hosts that can
/// report a failed/cancelled render should call [`try_gaussian_blur`] instead.
pub fn gaussian_blur_with_quality(img: &Image, sigma_x: f64, sigma_y: f64, repeat: bool, quality: Quality) -> Image {
    match try_gaussian_blur(img, sigma_x, sigma_y, repeat, quality, Cancel::Never) {
        Ok(image) => image,
        Err(error) => {
            log::error!("Gaussian render failed: {error}");
            img.clone()
        }
    }
}

/// The shared calibrated sampled kernel, with no box approximation or recursive tail.
pub fn gaussian_blur_exact(img: &Image, sigma_x: f64, sigma_y: f64, repeat: bool) -> Image {
    gaussian_blur_with_quality(img, sigma_x, sigma_y, repeat, Quality::Exact)
}

/// Fallible, cancellable Gaussian. Every recursive worker owns complete lines, not independently
/// initialized tiles. Scratch is reused across row batches/column stripes within each worker.
/// A cancellation discards the partially filtered copy and never changes `img`.
pub fn try_gaussian_blur(img: &Image, sigma_x: f64, sigma_y: f64, repeat: bool, quality: Quality, cancel: Cancel<'_>) -> Result<Image, Error> {
    let shape = Shape::new(img.width as usize, img.height as usize, 4)?;
    if img.data.as_flattened().len() != shape.samples() {
        return Err(Error::InvalidDimensions);
    }
    if cancel.cancelled() {
        return Err(Error::Cancelled);
    }
    let sigma = |value: f64| if value.is_nan() || value <= 1e-3 { 0.0 } else { value as f32 };
    let horizontal = Gaussian::new(sigma(sigma_x), quality)?;
    let vertical = Gaussian::new(sigma(sigma_y), quality)?;
    let mut output = img.clone();
    if img.is_empty() {
        return Ok(output);
    }
    let edge = if repeat { Edge::Clamp } else { Edge::Transparent };
    let width = shape.width();
    if horizontal.sigma() > 0.0 {
        let batch = width.checked_mul(LINE_BATCH * 8).ok_or(Error::SizeOverflow)?;
        output.data.par_chunks_mut(batch).try_for_each_init(Scratch::default, |scratch, rows| {
            let rows_shape = Shape::new(width, rows.len() / width, 4)?;
            horizontal.filter_rows(rows.as_flattened_mut(), rows_shape, edge, scratch, cancel)
        })?;
    }
    if vertical.sigma() > 0.0 {
        let mut stripes: Vec<Vec<&mut [f32]>> = (0..width.div_ceil(64)).map(|_| Vec::with_capacity(shape.height())).collect();
        for row in output.data.chunks_mut(width) {
            for (stripe, pixels) in stripes.iter_mut().zip(row.chunks_mut(64)) {
                stripe.push(pixels.as_flattened_mut());
            }
        }
        stripes.par_iter_mut().try_for_each_init(Scratch::default, |scratch, rows| vertical.filter_columns(rows, 4, edge, scratch, cancel))?;
    }
    Ok(output)
}

/// Motion blur along `angle_deg` (0 = vertical in AE's Directional Blur, measured clockwise from
/// up) over `length` pixels.
pub fn directional_blur(img: &Image, angle_deg: f64, length: f64) -> Image {
    if length < 0.5 {
        return img.clone();
    }
    let a = angle_deg.to_radians();
    let (dx, dy) = (a.sin(), -a.cos());
    let n = (length.ceil() as usize * 2 + 1).clamp(3, 257);
    let mut out = Image::new(img.width, img.height);
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let mut acc = [0.0f32; 4];
            for i in 0..n {
                let t = (i as f64 / (n - 1) as f64 - 0.5) * length;
                let p = img.sample_bilinear(x as f64 + 0.5 + dx * t, y as f64 + 0.5 + dy * t);
                for c in 0..4 {
                    acc[c] += p[c];
                }
            }
            *px = acc.map(|v| v / n as f32);
        }
    });
    out
}

/// Radial blur around `center`: `spin` (degrees) or zoom (`amount` as a fraction of distance).
pub fn radial_blur(img: &Image, center: (f64, f64), amount: f64, zoom: bool) -> Image {
    if amount.abs() < 1e-6 {
        return img.clone();
    }
    let n = 32usize;
    let mut out = Image::new(img.width, img.height);
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (vx, vy) = (x as f64 + 0.5 - center.0, y as f64 + 0.5 - center.1);
            let mut acc = [0.0f32; 4];
            for i in 0..n {
                let t = i as f64 / (n - 1) as f64 - 0.5;
                let (sx, sy) = if zoom {
                    let k = 1.0 + t * amount;
                    (center.0 + vx * k, center.1 + vy * k)
                } else {
                    let (s, c) = (t * amount).to_radians().sin_cos();
                    (center.0 + vx * c - vy * s, center.1 + vx * s + vy * c)
                };
                // Samples outside the image repeat its edge: After Effects' Radial Blur keeps a
                // layer's edges (cases 713, 829) instead of fading them toward transparency.
                let p = img.sample_bilinear(sx.clamp(0.5, (img.width as f64 - 0.5).max(0.5)), sy.clamp(0.5, (img.height as f64 - 0.5).max(0.5)));
                for ch in 0..4 {
                    acc[ch] += p[ch];
                }
            }
            *px = acc.map(|v| v / n as f32);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_variance(sigma: f64, vertical: bool, quality: Quality) -> f64 {
        let extent = (sigma * 32.0).max(64.0) as u32;
        let length = extent * 2 + 1;
        let mut image = if vertical { Image::new(1, length) } else { Image::new(length, 1) };
        image.data[extent as usize] = [0.25, 0.5, 1.0, 1.0];
        let (sigma_x, sigma_y) = if vertical { (0.0, sigma) } else { (sigma, 0.0) };
        let output = try_gaussian_blur(&image, sigma_x, sigma_y, false, quality, Cancel::Never).unwrap();
        let mass: f64 = output.data.iter().map(|pixel| f64::from(pixel[3])).sum();
        assert!((mass - 1.0).abs() < 1e-4, "sigma {sigma}: mass {mass}");
        output.data.iter().enumerate().map(|(index, pixel)| (index as f64 - f64::from(extent)).powi(2) * f64::from(pixel[3])).sum::<f64>() / mass
    }

    #[test]
    fn gaussian_variance_is_exact_on_both_axes() {
        for sigma in [1.0, 4.0, 16.0, 64.0] {
            for vertical in [false, true] {
                let variance = impulse_variance(sigma, vertical, Quality::Recursive);
                let error = (variance / sigma.powi(2) - 1.0).abs();
                println!("sigma={sigma}, vertical={vertical}, variance={variance:.9}, relative_error={error:.9}");
                assert!(error < 0.005, "sigma {sigma}, vertical {vertical}: {variance}");
            }
        }
    }

    #[test]
    fn box_compatibility_retains_the_documented_variance_deficit() {
        for sigma in [1.0, 4.0, 16.0, 64.0] {
            let variance = impulse_variance(sigma, false, Quality::BoxApprox { passes: 3 });
            assert!((variance / sigma.powi(2) - (1.0 - 1.0 / (3.0 * sigma))).abs() < 1e-5);
        }
    }

    #[test]
    fn parallel_gaussian_matches_shared_serial_kernel() {
        let mut image = Image::new(67, 23);
        for (index, pixel) in image.data.iter_mut().enumerate() {
            *pixel = [(index % 17) as f32 - 4.0, 0.25, 2.0, 0.5];
        }
        for sigma in [1.0, 4.0, 16.0, 64.0] {
            for repeat in [false, true] {
                let mut serial = image.clone();
                let shape = Shape::new(67, 23, 4).unwrap();
                let edge = if repeat { Edge::Clamp } else { Edge::Transparent };
                Gaussian::new(sigma as f32, Quality::Recursive)
                    .unwrap()
                    .apply(serial.data.as_flattened_mut(), shape, edge, &mut Scratch::default(), Cancel::Never)
                    .unwrap();
                assert_eq!(gaussian_blur(&image, sigma, sigma, repeat), serial, "sigma {sigma}, repeat {repeat}");
            }
        }
    }

    #[test]
    fn gaussian_errors_and_cancellation_never_modify_the_input() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let image = Image::filled(33, 31, [0.25, 0.5, 2.0, 0.5]);
        let original = image.clone();
        let calls = AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 2;
        assert_eq!(try_gaussian_blur(&image, 4.0, 4.0, true, Quality::Recursive, Cancel::Hook(&cancelled)), Err(Error::Cancelled));
        assert_eq!(image, original);
        assert_eq!(try_gaussian_blur(&image, f64::INFINITY, 0.0, true, Quality::Recursive, Cancel::Never), Err(Error::InvalidSigma));
        let malformed = Image { width: 100, height: 100, data: vec![[0.0; 4]] };
        assert_eq!(try_gaussian_blur(&malformed, 4.0, 4.0, true, Quality::Recursive, Cancel::Never), Err(Error::InvalidDimensions));
        assert_eq!(gaussian_blur(&Image::new(0, 4), 4.0, 4.0, false), Image::new(0, 4));
    }

    #[test]
    fn gaussian_preserves_mass() {
        let mut img = Image::new(64, 64);
        img.set(32, 32, [1.0; 4]);
        let b = gaussian_blur(&img, 4.0, 4.0, false);
        let sum: f32 = b.data.iter().map(|p| p[3]).sum();
        assert!((sum - 1.0).abs() < 1e-3, "{sum}");
        assert!(b.get(32, 32)[3] < 0.05);
    }

    #[test]
    fn vanishing_or_nan_sigma_leaves_the_image_unchanged() {
        let mut img = Image::new(5, 5);
        img.set(2, 2, [1.0, 0.5, 0.25, 1.0]);
        for s in [1e-200, f64::MIN_POSITIVE, f64::NAN] {
            let b = gaussian_blur_exact(&img, s, s, false);
            assert!(b.data.iter().all(|p| p.iter().all(|v| v.is_finite())), "sigma {s}");
            assert_eq!(b, img, "sigma {s}");
        }
    }

    #[test]
    fn flat_image_stays_flat_with_repeat() {
        let img = Image::filled(20, 10, [0.5, 0.5, 0.5, 1.0]);
        let b = gaussian_blur(&img, 3.0, 3.0, true);
        assert!(b.data.iter().all(|p| (p[0] - 0.5).abs() < 1e-5));
    }

    /// Direct (windowed sum) box filter along x or y: the definition the passes must match.
    fn box_ref(img: &Image, r: usize, vertical: bool, repeat: bool) -> Image {
        let mut out = Image::new(img.width, img.height);
        let (w, h) = (img.width as i64, img.height as i64);
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f64; 4];
                for k in -(r as i64)..=(r as i64) {
                    let (sx, sy) = if vertical { (x, y + k) } else { (x + k, y) };
                    let p = if repeat { img.get_clamped(sx, sy) } else { img.get(sx, sy) };
                    for c in 0..4 {
                        acc[c] += p[c] as f64;
                    }
                }
                out.set(x as u32, y as u32, acc.map(|v| (v / (2 * r + 1) as f64) as f32));
            }
        }
        out
    }

    #[test]
    fn passes_match_direct_box_filter() {
        let mut img = Image::new(53, 301);
        for y in 0..301 {
            for x in 0..53 {
                let v = crate::hash_noise(x, y, 3);
                img.set(x, y, [v * 0.5, v * 0.25, v, v]);
            }
        }
        for repeat in [false, true] {
            for (rx, ry) in [(0, 3), (2, 0), (5, 9), (40, 70), (1, 200)] {
                let fast = box_passes(&img, &[rx, rx + 1], &[ry, ry / 2], repeat);
                let mut slow = img.clone();
                for r in [rx, rx + 1] {
                    slow = box_ref(&slow, r, false, repeat);
                }
                for r in [ry, ry / 2] {
                    slow = box_ref(&slow, r, true, repeat);
                }
                for (i, (p, q)) in fast.data.iter().zip(&slow.data).enumerate() {
                    for c in 0..4 {
                        assert!((p[c] - q[c]).abs() < 1e-4, "r {rx},{ry} repeat {repeat} px {i}: {p:?} vs {q:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn radii_sane() {
        let r = craft_blur::box_widths(10.0).unwrap().map(|width| width / 2);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|&v| (7..=10).contains(&v)), "{r:?}");
    }
}
