//! Bit-identity of the coded stream. Hashes were captured from the scalar distortion loops
//! (FNV-1a 64 of each packet, length mixed in) before `craft-distortion` replaced them:
//! 4 frames, 96×64, keyint 4, ConstantQ 100 at 8-bit and ConstantQ 80 at 10-bit.

use effectcraft_av1enc::{Encoder, EncoderConfig, Frame, RateControl};

fn mix(hash: &mut u64, data: &[u8]) {
    *hash ^= data.len() as u64;
    *hash = hash.wrapping_mul(0x100000001b3);
    for &byte in data {
        *hash ^= byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn sample(x: usize, y: usize, frame: usize, salt: u32, bits: u32) -> u16 {
    let x2 = x.wrapping_add(frame.wrapping_mul(3));
    let y2 = y.wrapping_add(frame);
    let value = (x2 as u32).wrapping_mul(17).wrapping_add((y2 as u32).wrapping_mul(29)).wrapping_add((frame as u32).wrapping_mul(131)).wrapping_add(salt);
    (value & ((1u32 << bits) - 1)) as u16
}

fn plane(w: usize, h: usize, frame: usize, salt: u32, bits: u32) -> Vec<u16> {
    let mut out = vec![0u16; w * h];
    for row in 0..h {
        for col in 0..w {
            out[row * w + col] = sample(col, row, frame, salt, bits);
        }
    }
    out
}

fn bitstream_hash(bits: u8) -> u64 {
    let (w, h, frames) = (96usize, 64usize, 4usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
    cfg.bit_depth = bits;
    cfg.keyint = 4;
    cfg.rate = RateControl::ConstantQ(if bits == 8 { 100 } else { 80 });
    let mut enc = Encoder::new(cfg).expect("config");
    let mut hash = 0xcbf29ce484222325u64;
    for frame in 0..frames {
        let y = plane(w, h, frame, 0xA11, bits as u32);
        let u = plane(w / 2, h / 2, frame, 0xC10, bits as u32);
        let v = plane(w / 2, h / 2, frame, 0xD00, bits as u32);
        let packet = enc.encode(&Frame { y: &y, u: &u, v: &v, y_stride: w, uv_stride: w / 2 });
        mix(&mut hash, &packet.data);
    }
    hash
}

#[test]
fn eight_bit_stream_matches_the_scalar_distortion_hash() {
    assert_eq!(bitstream_hash(8), 0x8e09a214816cc76d);
}

#[test]
fn ten_bit_stream_matches_the_scalar_distortion_hash() {
    assert_eq!(bitstream_hash(10), 0x82e8dc9bc62817c5);
}
