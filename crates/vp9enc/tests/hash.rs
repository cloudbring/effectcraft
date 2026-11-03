//! Bit-identity of the coded frame. The hash was captured from the scalar SAD/SSE loops
//! (FNV-1a 64 of each frame, length mixed in) before `craft-distortion` replaced them:
//! 4 frames, 96×64, keyframe interval 4, quality 80.

use effectcraft_vp9enc::{EncoderConfig, Vp9Encoder};

fn mix(hash: &mut u64, data: &[u8]) {
    *hash ^= data.len() as u64;
    *hash = hash.wrapping_mul(0x100000001b3);
    for &byte in data {
        *hash ^= byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn sample(x: usize, y: usize, frame: usize, salt: u32) -> u8 {
    let x2 = x.wrapping_add(frame.wrapping_mul(3));
    let y2 = y.wrapping_add(frame);
    let value = (x2 as u32).wrapping_mul(17).wrapping_add((y2 as u32).wrapping_mul(29)).wrapping_add((frame as u32).wrapping_mul(131)).wrapping_add(salt);
    (value & 255) as u8
}

fn plane(w: usize, h: usize, frame: usize, salt: u32) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for row in 0..h {
        for col in 0..w {
            out[row * w + col] = sample(col, row, frame, salt);
        }
    }
    out
}

#[test]
fn stream_matches_the_scalar_distortion_hash() {
    let (w, h, frames) = (96usize, 64usize, 4usize);
    let mut enc =
        Vp9Encoder::new(EncoderConfig { width: w as u32, height: h as u32, quality: 80, keyframe_interval: 4, frame_rate: 30.0, ..EncoderConfig::default() });
    let mut hash = 0xcbf29ce484222325u64;
    for frame in 0..frames {
        let y = plane(w, h, frame, 0xA11);
        let u = plane(w / 2, h / 2, frame, 0xC10);
        let v = plane(w / 2, h / 2, frame, 0xD00);
        mix(&mut hash, &enc.encode_yuv420(&y, &u, &v));
    }
    assert_eq!(hash, 0x6b6d7ffd4e5acc46);
}
