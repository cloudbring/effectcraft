//! 8-bit SAD, SATD and SSE for mode decision, through [`craft_distortion`].
//!
//! `satd_av1` is the EffectCraft convention: 8×8 Hadamard tiles divided by 4 when
//! the side is a multiple of 8, otherwise the halved 4×4 score. SAD and SSE are
//! plain sums. The shared kernels take 8-bit samples, so 10-bit blocks stay on
//! [`scalar`] (the loops that used to live on `Decider`). Callers clamp 8-bit
//! samples to `0..=255` before they get here, and these helpers read that low byte.

use craft_distortion::{sad, satd_av1, sse};

/// One plane window: `data[row * stride + col]`.
pub(crate) struct View<'a> {
    pub data: &'a [u16],
    pub stride: usize,
}

/// Integer reference window for `Decider::sad_ref` (luma, possibly outside the picture).
pub(crate) struct SadRef<'a> {
    pub src: &'a [u8],
    pub src_stride: usize,
    pub reference: &'a [u8],
    pub ref_stride: usize,
    pub rx: i32,
    pub ry: i32,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    /// Visible width and height. Outside samples clamp to this rectangle.
    pub visible: (i32, i32),
}

pub(crate) fn sad8(src: &[u8], src_stride: usize, pred: &[u16], pred_stride: usize, w: usize, h: usize) -> u64 {
    let mut packed = [0u8; 64 * 64];
    pack_pred(pred, pred_stride, w, h, &mut packed);
    sad(src, src_stride, &packed, w, w, h) as u64
}

pub(crate) fn satd8(src: &[u8], src_stride: usize, pred: &[u16], n: usize) -> u64 {
    let mut packed = [0u8; 64 * 64];
    pack_pred(pred, n, n, n, &mut packed);
    satd_av1(src, src_stride, &packed, n, n, n) as u64
}

pub(crate) fn sse8(src: &[u8], src_stride: usize, pred: &[u16], pred_stride: usize, w: usize, h: usize) -> f64 {
    let mut packed = [0u8; 64 * 64];
    pack_pred(pred, pred_stride, w, h, &mut packed);
    sse(src, src_stride, &packed, w, w, h) as f64
}

/// SAD of a luma block. Inside the picture this is one kernel call; outside, the
/// reference is clamped to the visible rectangle and then scored the same way.
pub(crate) fn sad_ref8(p: &SadRef<'_>) -> u64 {
    let SadRef { src, src_stride, reference, ref_stride, rx, ry, x, y, w, h, visible: (vw, vh) } = *p;
    let (lx, ly) = (vw - 1, vh - 1);
    let inside = rx >= 0 && ry >= 0 && rx + w as i32 - 1 <= lx && ry + h as i32 - 1 <= ly;
    if inside {
        return sad(&src[y * src_stride + x..], src_stride, &reference[ry as usize * ref_stride + rx as usize..], ref_stride, w, h) as u64;
    }
    let mut packed_src = [0u8; 64 * 64];
    let mut packed_ref = [0u8; 64 * 64];
    for i in 0..h {
        let s = (y + i) * src_stride + x;
        packed_src[i * w..i * w + w].copy_from_slice(&src[s..s + w]);
        let rr = (ry + i as i32).clamp(0, ly) as usize;
        for j in 0..w {
            let cx = (rx + j as i32).clamp(0, lx) as usize;
            packed_ref[i * w + j] = reference[rr * ref_stride + cx];
        }
    }
    sad(&packed_src, w, &packed_ref, w, w, h) as u64
}

/// Sum of squared errors over the rows whose superblock row is a multiple of `row_step`
/// (the loop-filter level search). 8-bit rows are tiled into 64×64 kernel calls.
pub(crate) fn sse_visible(bd: u32, rec: View<'_>, src: View<'_>, width: usize, height: usize, row_step: usize) -> u64 {
    if bd > 8 {
        return scalar::sse_rows(rec, src, width, height, row_step);
    }
    let mut total = 0u64;
    let mut y = 0;
    while y < height {
        if !(y >> 6).is_multiple_of(row_step) {
            y += 1;
            continue;
        }
        let start = y;
        y += 1;
        while y < height && y - start < 64 && (y >> 6).is_multiple_of(row_step) {
            y += 1;
        }
        total += sse_rect_u16(&rec.data[start * rec.stride..], rec.stride, &src.data[start * src.stride..], src.stride, width, y - start);
    }
    total
}

fn sse_rect_u16(a: &[u16], a_stride: usize, b: &[u16], b_stride: usize, width: usize, height: usize) -> u64 {
    let mut total = 0u64;
    let mut y = 0;
    while y < height {
        let bh = (height - y).min(64);
        let mut x = 0;
        while x < width {
            let bw = (width - x).min(64);
            let mut pa = [0u8; 64 * 64];
            let mut pb = [0u8; 64 * 64];
            for row in 0..bh {
                for col in 0..bw {
                    pa[row * bw + col] = a[(y + row) * a_stride + x + col] as u8;
                    pb[row * bw + col] = b[(y + row) * b_stride + x + col] as u8;
                }
            }
            total += sse(&pa, bw, &pb, bw, bw, bh);
            x += bw;
        }
        y += bh;
    }
    total
}

fn pack_pred(pred: &[u16], stride: usize, w: usize, h: usize, dst: &mut [u8]) {
    for row in 0..h {
        for col in 0..w {
            dst[row * w + col] = pred[row * stride + col] as u8;
        }
    }
}

/// 10-bit scores. The shared kernels are 8-bit; narrowing would change Main 10 decisions.
pub(crate) mod scalar {
    use super::View;

    pub(crate) fn sad(src: &[u16], src_stride: usize, pred: &[u16], pred_stride: usize, w: usize, h: usize) -> u64 {
        let mut s = 0u64;
        for i in 0..h {
            let row = &src[i * src_stride..i * src_stride + w];
            s += row.iter().zip(&pred[i * pred_stride..i * pred_stride + w]).map(|(&a, &b)| (a as i32 - b as i32).unsigned_abs()).sum::<u32>() as u64;
        }
        s
    }

    pub(crate) fn sse(src: &[u16], src_stride: usize, blk: &[u16], blk_stride: usize, w: usize, h: usize) -> u64 {
        let mut s = 0u64;
        for i in 0..h {
            let row = &src[i * src_stride..i * src_stride + w];
            s += row
                .iter()
                .zip(&blk[i * blk_stride..i * blk_stride + w])
                .map(|(&a, &b)| {
                    let d = a as i64 - b as i64;
                    (d * d) as u64
                })
                .sum::<u64>();
        }
        s
    }

    pub(crate) fn satd(src: &[u16], src_stride: usize, pred: &[u16], pred_stride: usize, n: usize) -> u64 {
        let k = n.min(8);
        let mut total = 0u64;
        let mut d = [0i32; 64];
        for by in (0..n).step_by(k) {
            for bx in (0..n).step_by(k) {
                for i in 0..k {
                    for j in 0..k {
                        let a = src[(by + i) * src_stride + bx + j];
                        let b = pred[(by + i) * pred_stride + bx + j];
                        d[i * k + j] = a as i32 - b as i32;
                    }
                }
                for i in 0..k {
                    hadamard(&mut d[i * k..i * k + k]);
                }
                let mut col = [0i32; 8];
                let mut s = 0u64;
                for j in 0..k {
                    for i in 0..k {
                        col[i] = d[i * k + j];
                    }
                    hadamard(&mut col[..k]);
                    s += col[..k].iter().map(|v| v.unsigned_abs() as u64).sum::<u64>();
                }
                total += s / (k as u64 / 2);
            }
        }
        total
    }

    fn hadamard(v: &mut [i32]) {
        let n = v.len();
        let mut h = 1;
        while h < n {
            for i in (0..n).step_by(2 * h) {
                for j in i..i + h {
                    let (a, b) = (v[j], v[j + h]);
                    v[j] = a + b;
                    v[j + h] = a - b;
                }
            }
            h *= 2;
        }
    }

    /// Edge-clamped reference SAD. `src` already starts at the block origin.
    /// This is the loop that used to live in `Decider::sad_ref` (10-bit still uses it).
    pub(crate) struct Clamped<'a> {
        pub src: &'a [u16],
        pub stride: usize,
        pub reference: &'a [u16],
        pub ref_stride: usize,
        pub rx: i32,
        pub ry: i32,
        pub w: usize,
        pub h: usize,
        pub visible: (i32, i32),
    }

    pub(crate) fn sad_ref(p: &Clamped<'_>) -> u64 {
        let (vw, vh) = p.visible;
        let (lx, ly) = (vw - 1, vh - 1);
        let mut total = 0u64;
        let inside = p.rx >= 0 && p.ry >= 0 && p.rx + p.w as i32 - 1 <= lx && p.ry + p.h as i32 - 1 <= ly;
        for i in 0..p.h {
            let s = &p.src[i * p.stride..i * p.stride + p.w];
            let rr = (p.ry + i as i32).clamp(0, ly) as usize;
            let row = &p.reference[rr * p.ref_stride..rr * p.ref_stride + vw as usize];
            if inside {
                let start = p.rx as usize;
                total += s.iter().zip(&row[start..start + p.w]).map(|(&a, &b)| (a as i32 - b as i32).unsigned_abs()).sum::<u32>() as u64;
            } else {
                for j in 0..p.w {
                    let b = row[(p.rx + j as i32).clamp(0, lx) as usize];
                    total += (s[j] as i32 - b as i32).unsigned_abs() as u64;
                }
            }
        }
        total
    }

    /// Rows of `rec` against `src` whose superblock row `(y >> 6)` is a multiple of `row_step`.
    pub(crate) fn sse_rows(rec: View<'_>, src: View<'_>, width: usize, height: usize, row_step: usize) -> u64 {
        let mut s = 0u64;
        for y in 0..height {
            if !(y >> 6).is_multiple_of(row_step) {
                continue;
            }
            for (a, b) in rec.data[y * rec.stride..y * rec.stride + width].iter().zip(&src.data[y * src.stride..y * src.stride + width]) {
                let d = *a as i64 - *b as i64;
                s += (d * d) as u64;
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_distortion::{sad, satd_av1, sse};

    /// Scalar SAD copied from `Decider::sad` before it called `craft_distortion`.
    fn sad_oracle(src: &[u16], src_stride: usize, pred: &[u16], pred_stride: usize, w: usize, h: usize) -> u64 {
        scalar::sad(src, src_stride, pred, pred_stride, w, h)
    }

    /// Scalar SSE copied from `Decider::sse` (the integer sum, before the `f64` cast).
    fn sse_oracle(src: &[u16], src_stride: usize, blk: &[u16], blk_stride: usize, w: usize, h: usize) -> u64 {
        scalar::sse(src, src_stride, blk, blk_stride, w, h)
    }

    /// Scalar SATD copied from `Decider::satd` / `hadamard` (8×8 tiles divided by 4,
    /// 4×4 tiles divided by 2).
    fn satd_oracle(src: &[u16], src_stride: usize, pred: &[u16], pred_stride: usize, n: usize) -> u64 {
        scalar::satd(src, src_stride, pred, pred_stride, n)
    }

    fn next(state: &mut u32) -> u32 {
        *state ^= state.wrapping_shl(13);
        *state ^= *state >> 17;
        *state ^= state.wrapping_shl(5);
        *state
    }

    fn fill(state: &mut u32, buf: &mut [u16], stride: usize, w: usize, h: usize, bits: u32) {
        let mask = (1u32 << bits) - 1;
        for row in 0..h {
            for col in 0..stride {
                let v = next(state) & mask;
                buf[row * stride + col] = if col < w { v as u16 } else { 0 };
            }
        }
    }

    #[test]
    fn eight_bit_matches_scalar_oracles_on_random_blocks() {
        let mut state = 0x99u32;
        let shapes: &[(usize, usize)] = &[(1, 1), (3, 5), (4, 4), (7, 13), (8, 8), (15, 16), (16, 9), (17, 19), (32, 32), (64, 1), (1, 64), (64, 64), (6, 6)];
        for &(w, h) in shapes {
            for _ in 0..6 {
                let stride_a = w + (next(&mut state) as usize % 5);
                let stride_b = w + (next(&mut state) as usize % 5);
                let mut a = vec![0u16; stride_a * h];
                let mut b = vec![0u16; stride_b * h];
                fill(&mut state, &mut a, stride_a, w, h, 8);
                fill(&mut state, &mut b, stride_b, w, h, 8);
                let au8: Vec<u8> = a.iter().map(|&s| s as u8).collect();
                assert_eq!(
                    sad(&au8, stride_a, &b.iter().map(|&s| s as u8).collect::<Vec<_>>(), stride_b, w, h) as u64,
                    sad_oracle(&a, stride_a, &b, stride_b, w, h),
                    "sad {w}x{h}"
                );
                assert_eq!(
                    sse(&au8, stride_a, &b.iter().map(|&s| s as u8).collect::<Vec<_>>(), stride_b, w, h),
                    sse_oracle(&a, stride_a, &b, stride_b, w, h),
                    "sse {w}x{h}"
                );
                let _ = au8;
            }
        }
        for n in [4usize, 8, 16, 32, 64] {
            for _ in 0..6 {
                let sa = n + (next(&mut state) as usize % 4);
                let sb = n + (next(&mut state) as usize % 4);
                let mut a = vec![0u16; sa * n];
                let mut b = vec![0u16; sb * n];
                fill(&mut state, &mut a, sa, n, n, 8);
                fill(&mut state, &mut b, sb, n, n, 8);
                let au8: Vec<u8> = a.iter().map(|&s| s as u8).collect();
                let bu8: Vec<u8> = b.iter().map(|&s| s as u8).collect();
                assert_eq!(satd_av1(&au8, sa, &bu8, sb, n, n) as u64, satd_oracle(&a, sa, &b, sb, n), "satd {n}");
            }
        }
    }

    /// Scalar edge-clamped SAD copied from `Decider::sad_ref`.
    fn sad_ref_oracle(block: &scalar::Clamped<'_>) -> u64 {
        scalar::sad_ref(block)
    }

    #[test]
    fn clamped_reference_sad_matches_the_scalar_loop() {
        let (vw, vh) = (24i32, 18i32);
        let stride = 32usize;
        let mut state = 0x51u32;
        let mut src = vec![0u16; stride * vh as usize];
        let mut reference = vec![0u16; stride * vh as usize];
        fill(&mut state, &mut src, stride, vw as usize, vh as usize, 8);
        fill(&mut state, &mut reference, stride, vw as usize, vh as usize, 8);
        let src8: Vec<u8> = src.iter().map(|&s| s as u8).collect();
        let ref8: Vec<u8> = reference.iter().map(|&s| s as u8).collect();
        for n in [4usize, 8, 16] {
            for &(x, y, rx, ry) in &[(0, 0, 0, 0), (4, 2, -3, 1), (8, 6, 20, -4), (16, 10, 30, 20), (0, 0, 5, -2)] {
                let w = n.min((vw as usize).saturating_sub(x));
                let h = n.min((vh as usize).saturating_sub(y));
                if w == 0 || h == 0 {
                    continue;
                }
                let got = sad_ref8(&SadRef { src: &src8, src_stride: stride, reference: &ref8, ref_stride: stride, rx, ry, x, y, w, h, visible: (vw, vh) });
                let origin = y * stride + x;
                let expect = sad_ref_oracle(&scalar::Clamped {
                    src: &src[origin..],
                    stride,
                    reference: &reference,
                    ref_stride: stride,
                    rx,
                    ry,
                    w,
                    h,
                    visible: (vw, vh),
                });
                assert_eq!(got, expect, "sad_ref n={n} at {x},{y} mv {rx},{ry}");
            }
        }
    }
}
