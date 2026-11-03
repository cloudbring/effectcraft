//! 8-bit SAD and SATD for motion search and intra mode decision, through [`craft_distortion`].
//!
//! `satd_hevc` matches the loops that used to live in `frame`: 8×8 tiles use `(sum + 2) >> 2`,
//! and 4×4 uses the halved Hadamard (`(sum + 1) >> 1`, equal to a truncating divide by 2 because
//! that absolute sum is even). SAD is an unscaled sum. 10-bit blocks stay on [`scalar`]; the
//! shared kernels take 8-bit samples.

use craft_distortion::{sad as sad_kernel, satd_hevc};

/// HEVC SATD of an `n`×`n` block (`n` is 4, 8, 16, 32 or 64).
pub(crate) fn satd(bd: u32, src: &[u16], ss: usize, pred: &[u16], ps: usize, n: usize) -> u32 {
    if bd <= 8 {
        let mut a = [0u8; 64 * 64];
        let mut b = [0u8; 64 * 64];
        for row in 0..n {
            for col in 0..n {
                a[row * n + col] = src[row * ss + col] as u8;
                b[row * n + col] = pred[row * ps + col] as u8;
            }
        }
        return satd_hevc(&a, n, &b, n, n, n);
    }
    scalar::satd(src, ss, pred, ps, n)
}

/// Integer-position luma SAD of an `n`×`n` block, 8-bit planes.
pub(crate) fn sad8(src: &[u8], ss: usize, reference: &[u8], rs: usize, n: usize) -> u32 {
    sad_kernel(src, ss, reference, rs, n, n)
}

/// Integer-position luma SAD of an `n`×`n` block, samples wider than 8 bit.
pub(crate) fn sad10(src: &[u16], ss: usize, reference: &[u16], rs: usize, n: usize) -> u32 {
    scalar::sad(src, ss, reference, rs, n)
}

/// 10-bit scores, the scalar loops removed from `frame::satd` and `MeRef::sad`.
pub(crate) mod scalar {
    pub(crate) fn satd(src: &[u16], ss: usize, pred: &[u16], ps: usize, n: usize) -> u32 {
        if n == 4 {
            return satd4(src, ss, pred, ps);
        }
        let mut total = 0;
        for by in (0..n).step_by(8) {
            for bx in (0..n).step_by(8) {
                total += satd8(&src[by * ss + bx..], ss, &pred[by * ps + bx..], ps);
            }
        }
        total
    }

    fn satd4(src: &[u16], ss: usize, pred: &[u16], ps: usize) -> u32 {
        let mut d = [0i32; 16];
        for y in 0..4 {
            for x in 0..4 {
                d[y * 4 + x] = src[y * ss + x] as i32 - pred[y * ps + x] as i32;
            }
        }
        let mut m = [0i32; 16];
        for y in 0..4 {
            let r = &d[y * 4..y * 4 + 4];
            let (a, b, c, e) = (r[0] + r[3], r[1] + r[2], r[1] - r[2], r[0] - r[3]);
            m[y * 4] = a + b;
            m[y * 4 + 1] = e + c;
            m[y * 4 + 2] = a - b;
            m[y * 4 + 3] = e - c;
        }
        let mut sum = 0;
        for x in 0..4 {
            let (r0, r1, r2, r3) = (m[x], m[4 + x], m[8 + x], m[12 + x]);
            let (a, b, c, e) = (r0 + r3, r1 + r2, r1 - r2, r0 - r3);
            sum += (a + b).abs() + (e + c).abs() + (a - b).abs() + (e - c).abs();
        }
        ((sum + 1) >> 1) as u32
    }

    fn hadamard8(v: &mut [i32; 8]) {
        let mut t = *v;
        let mut h = 1;
        while h < 8 {
            for i in (0..8).step_by(h * 2) {
                for j in i..i + h {
                    let (a, b) = (t[j], t[j + h]);
                    t[j] = a + b;
                    t[j + h] = a - b;
                }
            }
            h *= 2;
        }
        *v = t;
    }

    fn satd8(src: &[u16], ss: usize, pred: &[u16], ps: usize) -> u32 {
        let mut m = [[0i32; 8]; 8];
        for y in 0..8 {
            for x in 0..8 {
                m[y][x] = src[y * ss + x] as i32 - pred[y * ps + x] as i32;
            }
            hadamard8(&mut m[y]);
        }
        let mut sum = 0;
        for x in 0..8 {
            let mut col = [0i32; 8];
            for y in 0..8 {
                col[y] = m[y][x];
            }
            hadamard8(&mut col);
            sum += col.iter().map(|v| v.abs()).sum::<i32>();
        }
        ((sum + 2) >> 2) as u32
    }

    pub(crate) fn sad(src: &[u16], ss: usize, reference: &[u16], rs: usize, n: usize) -> u32 {
        let mut s = 0u32;
        for r in 0..n {
            let a = &src[r * ss..r * ss + n];
            let b = &reference[r * rs..r * rs + n];
            s += a.iter().zip(b).map(|(&x, &y)| (x as i32 - y as i32).unsigned_abs()).sum::<u32>();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_distortion::{sad as sad_kernel, satd_hevc};

    /// Scalar SATD copied from `frame::satd` / `satd4` / `satd8` before `craft_distortion`.
    fn satd_oracle(src: &[u16], ss: usize, pred: &[u16], ps: usize, n: usize) -> u32 {
        scalar::satd(src, ss, pred, ps, n)
    }

    /// Scalar SAD copied from `MeRef::sad`.
    fn sad_oracle(src: &[u16], ss: usize, reference: &[u16], rs: usize, n: usize) -> u32 {
        scalar::sad(src, ss, reference, rs, n)
    }

    fn next(state: &mut u32) -> u32 {
        *state ^= state.wrapping_shl(13);
        *state ^= *state >> 17;
        *state ^= state.wrapping_shl(5);
        *state
    }

    #[test]
    fn eight_bit_matches_scalar_oracles_on_random_blocks() {
        let mut state = 0x265u32;
        for n in [4usize, 8, 16, 32, 64] {
            for _ in 0..8 {
                let ss = n + (next(&mut state) as usize % 3);
                let ps = n + (next(&mut state) as usize % 3);
                let mut src = vec![0u16; ss * n];
                let mut pred = vec![0u16; ps * n];
                for row in 0..n {
                    for col in 0..n {
                        src[row * ss + col] = (next(&mut state) & 255) as u16;
                        pred[row * ps + col] = (next(&mut state) & 255) as u16;
                    }
                }
                let src8: Vec<u8> = src.iter().map(|&s| s as u8).collect();
                let pred8: Vec<u8> = pred.iter().map(|&s| s as u8).collect();
                assert_eq!(satd_hevc(&src8, ss, &pred8, ps, n, n), satd_oracle(&src, ss, &pred, ps, n), "satd {n}");
                assert_eq!(sad_kernel(&src8, ss, &pred8, ps, n, n), sad_oracle(&src, ss, &pred, ps, n), "sad {n}");
            }
        }
    }
}
