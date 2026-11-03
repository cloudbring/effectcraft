//! Bit-identity of the coded access unit. Hashes were captured from the scalar distortion loops
//! (FNV-1a 64 of each packet, length mixed in) before `craft-distortion` replaced them:
//! 4 frames, 96×64, keyint 4, ConstantQp 28 on Main and ConstantQp 24 on Main 10.

use effectcraft_hevcenc::{Encoder, EncoderConfig, Frame, Profile, RateControl};

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

fn bitstream_hash(main10: bool) -> u64 {
    let (w, h, frames) = (96usize, 64usize, 4usize);
    let bits = if main10 { 10 } else { 8 };
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
    cfg.profile = if main10 { Profile::Main10 } else { Profile::Main };
    cfg.keyint = 4;
    cfg.rate = RateControl::ConstantQp(if main10 { 24 } else { 28 });
    let mut enc = Encoder::new(cfg).expect("config");
    let mut hash = 0xcbf29ce484222325u64;
    for frame in 0..frames {
        let y = plane(w, h, frame, 0xA11, bits);
        let u = plane(w / 2, h / 2, frame, 0xC10, bits);
        let v = plane(w / 2, h / 2, frame, 0xD00, bits);
        let packet = enc.encode(&Frame { y: &y, u: &u, v: &v, y_stride: w, uv_stride: w / 2 });
        mix(&mut hash, &packet.data);
    }
    hash
}

#[test]
fn main_stream_matches_the_scalar_distortion_hash() {
    assert_eq!(bitstream_hash(false), 0x219b41a4f2acfb2a);
}

#[test]
fn main10_stream_matches_the_scalar_distortion_hash() {
    assert_eq!(bitstream_hash(true), 0xc0b98172fe0726d1);
}
