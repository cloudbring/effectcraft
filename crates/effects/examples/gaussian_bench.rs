use std::hint::black_box;
use std::time::Instant;

use effectcraft_effects::{Buf, EffectCtx, EffectEnv, Params, apply, default_value, find};
use effectcraft_keyframe::Value;
use effectcraft_raster::{Image, gaussian_blur};

fn main() {
    for sigma in [1.0, 4.0, 16.0, 64.0] {
        let center = (32.0_f64 * sigma).max(64.0) as u32;
        let mut impulse = Image::new(center * 2 + 1, 1);
        impulse.set(center, 0, [1.0; 4]);
        let blurred = gaussian_blur(&impulse, sigma, 0.0, false);
        let support = center as usize;
        let taps = &blurred.data[center as usize - support..=center as usize + support];
        let mass: f64 = taps.iter().map(|pixel| f64::from(pixel[0])).sum();
        let variance: f64 = taps.iter().enumerate().map(|(index, pixel)| (index as f64 - support as f64).powi(2) * f64::from(pixel[0])).sum::<f64>() / mass;
        println!("sigma={sigma} variance={variance:.9} error={:.6}%", 100.0 * (variance / sigma.powi(2) - 1.0));
    }
    let Some(spec) = find("ec.blur.gaussian") else {
        eprintln!("Gaussian effect is unavailable");
        return;
    };
    let mut image = Image::new(1920, 1080);
    for (index, pixel) in image.data.iter_mut().enumerate() {
        let alpha = 0.25 + (index % 257) as f32 / 512.0;
        *pixel = [alpha * (index % 113) as f32 / 113.0, alpha * (index % 97) as f32 / 97.0, alpha * 0.4, alpha];
    }
    println!("1080p Gaussian effect; rayon threads={}", rayon::current_num_threads());
    for sigma in [2.0, 16.0, 64.0] {
        let mut params = Params { values: spec.params.iter().map(|param| (param.id.to_string(), default_value(param, [1920.0, 1080.0]))).collect() };
        params.values.insert("blurriness".into(), Value::Scalar(sigma * 2.0));
        params.values.insert("repeatEdge".into(), Value::Bool(true));
        let context = EffectCtx { params: &params, time: 0.0, layer_size: [1920.0, 1080.0], seed: 0, adjustment: false, env: EffectEnv::default() };
        let mut samples = Vec::new();
        for iteration in 0..9 {
            let buffer = Buf { img: image.clone(), offset: [0.0; 2], scale: 1.0 };
            let start = Instant::now();
            let output = apply(spec, &context, buffer);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            black_box(output);
            if iteration >= 2 {
                samples.push(elapsed);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!("sigma={sigma} median_ms={:.3} samples={samples:?}", samples[samples.len() / 2]);
    }
}
