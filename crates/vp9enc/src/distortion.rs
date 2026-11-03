//! 8-bit SAD and SSE for VP9 motion search and loop-filter selection, through [`craft_distortion`].
//!
//! VP9 does not use SATD. SAD and SSE are unscaled sums. Full-frame SSE is tiled at 64 samples
//! because the kernel rejects a side longer than [`craft_distortion::MAX_SIDE`].

use craft_distortion::{MAX_SIDE, sad, sse};

/// SAD of an `n`×`n` block. A kernel rejection (`u32::MAX`, the old out-of-range sentinel
/// was `u32::MAX / 4`) is reported as that sentinel so a short plane still loses the search.
pub(crate) fn sad_block(a: &[u8], a_stride: usize, b: &[u8], b_stride: usize, n: usize) -> u32 {
    let score = sad(a, a_stride, b, b_stride, n, n);
    if score == u32::MAX { u32::MAX / 4 } else { score }
}

/// Sum of squared errors over a `width`×`height` window, strided.
pub(crate) fn sse_area(src: &[u8], rec: &[u8], stride: usize, width: usize, height: usize) -> u64 {
    let mut total = 0u64;
    let mut y = 0;
    while y < height {
        let bh = (height - y).min(MAX_SIDE);
        let mut x = 0;
        while x < width {
            let bw = (width - x).min(MAX_SIDE);
            total += sse(&src[y * stride + x..], stride, &rec[y * stride + x..], stride, bw, bh);
            x += bw;
        }
        y += bh;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::{sad_block, sse_area};

    /// Scalar SAD copied from `tile::sad_n` (and the generic `n` arm of `Tile::sad`).
    fn sad_oracle(a: &[u8], sa: usize, b: &[u8], sb: usize, n: usize) -> u32 {
        let mut acc = 0u32;
        for i in 0..n {
            let (Some(ra), Some(rb)) = (a.get(i * sa..i * sa + n), b.get(i * sb..i * sb + n)) else {
                return u32::MAX / 4;
            };
            acc += ra.iter().zip(rb).map(|(p, q)| p.abs_diff(*q) as u32).sum::<u32>();
        }
        acc
    }

    /// Scalar SSE copied from `choose_level`'s luma error.
    fn sse_oracle(src: &[u8], rec: &[u8], stride: usize, width: usize, height: usize) -> u64 {
        let mut s = 0u64;
        for y in 0..height {
            for x in 0..width {
                let d = src[y * stride + x] as i64 - rec[y * stride + x] as i64;
                s += (d * d) as u64;
            }
        }
        s
    }

    fn next(state: &mut u32) -> u32 {
        *state ^= state.wrapping_shl(13);
        *state ^= *state >> 17;
        *state ^= state.wrapping_shl(5);
        *state
    }

    #[test]
    fn matches_scalar_oracles_on_random_blocks() {
        let mut state = 0x9u32;
        for n in [4usize, 8, 16, 32, 64] {
            for _ in 0..8 {
                let sa = n + (next(&mut state) as usize % 5);
                let sb = n + (next(&mut state) as usize % 5);
                let mut a = vec![0u8; sa * n];
                let mut b = vec![0u8; sb * n];
                for sample in a.iter_mut().chain(b.iter_mut()) {
                    *sample = (next(&mut state) & 255) as u8;
                }
                assert_eq!(sad_block(&a, sa, &b, sb, n), sad_oracle(&a, sa, &b, sb, n), "sad {n}");
            }
        }
        // Wider than one kernel tile, so the SSE split has to add up.
        for &(w, h) in &[(8usize, 8), (64, 64), (80, 70), (1920, 4), (3, 100)] {
            let stride = w + 8;
            let mut src = vec![0u8; stride * h];
            let mut rec = vec![0u8; stride * h];
            for sample in src.iter_mut().chain(rec.iter_mut()) {
                *sample = (next(&mut state) & 255) as u8;
            }
            assert_eq!(sse_area(&src, &rec, stride, w, h), sse_oracle(&src, &rec, stride, w, h), "sse {w}x{h}");
        }
    }
}
