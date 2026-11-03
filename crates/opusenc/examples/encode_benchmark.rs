use std::hint::black_box;
use std::time::Instant;

use effectcraft_opusenc::OpusEncoder;

fn main() {
    let pcm: Vec<f32> = (0..60 * 48_000)
        .flat_map(|index| {
            let time = index as f64 / 48_000.0;
            [
                (0.3 * (std::f64::consts::TAU * 440.0 * time).sin() + 0.1 * (std::f64::consts::TAU * 1320.0 * time).sin()) as f32,
                (0.25 * (std::f64::consts::TAU * 659.25 * time).sin() + 0.12 * (std::f64::consts::TAU * 1977.75 * time).sin()) as f32,
            ]
        })
        .collect();
    let mut warmup = OpusEncoder::new(2, 128_000);
    for frame in pcm.as_chunks::<{ OpusEncoder::FRAME_SIZE * 2 }>().0.iter().take(50) {
        black_box(warmup.encode_float(black_box(frame)));
    }
    for trial in 1..=3 {
        let mut encoder = OpusEncoder::new(2, 128_000);
        let started = Instant::now();
        let mut bytes = 0;
        for frame in pcm.as_chunks::<{ OpusEncoder::FRAME_SIZE * 2 }>().0 {
            bytes += black_box(encoder.encode_float(black_box(frame))).len();
        }
        println!("trial={trial} seconds={:.6} bytes={bytes} audio=60s stereo=48000Hz bitrate=128000", started.elapsed().as_secs_f64());
    }
}
