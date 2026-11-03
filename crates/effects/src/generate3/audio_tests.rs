use super::*;

#[test]
fn hann_magnitudes_match_direct_dft_at_every_supported_size() {
    let mut max_delta = 0.0f64;
    for exponent in 6..=16 {
        let length = 1usize << exponent;
        for count in [length - 13, length] {
            let samples: Vec<f64> = (0..count).map(|index| ((index * 17 % 101) as f64 - 50.0) / 50.0).collect();
            compute_fft_mags(&samples, |hz_per_bin, magnitudes| {
                assert_eq!(hz_per_bin, AUDIO_RATE as f64 / length as f64);
                assert_eq!(magnitudes.len(), length / 2 + 1);
                let bins: Vec<usize> = if length <= 256 { (0..=length / 2).collect() } else { vec![0, 1, 7, length / 4, length / 2 - 1, length / 2] };
                for bin in bins {
                    let mut expected = Complex::new(0.0, 0.0);
                    for (index, sample) in samples.iter().enumerate() {
                        let window = 0.5 - 0.5 * (2.0 * PI * index as f64 / (count - 1) as f64).cos();
                        let angle = -2.0 * PI * bin as f64 * index as f64 / length as f64;
                        expected += Complex::new(angle.cos(), angle.sin()) * sample * window;
                    }
                    let expected = expected.norm() * 4.0 / count as f64;
                    let delta = (magnitudes[bin] - expected).abs();
                    max_delta = max_delta.max(delta);
                    assert!(delta < 1e-9, "length {length}, count {count}, bin {bin}: {delta:e}");
                }
            })
            .unwrap();
        }
    }
    println!("maximum Hann magnitude delta against direct DFT: {max_delta:e}");
}

#[test]
fn cached_analysis_clears_padding_and_previous_frames() {
    for count in [0, 1, 2, 16, 63, 64, 65, 127, 128, 129, 8191, 65535] {
        let signal = vec![1.0; count];
        let zeros = vec![0.0; count];
        compute_fft_mags(&signal, |_, _| {}).unwrap();
        compute_fft_mags(&zeros, |_, magnitudes| assert!(magnitudes.iter().all(|&value| value == 0.0))).unwrap();
    }
    let mut fft = SpectrumFft::new(128).unwrap();
    fft.analyze(&[1.0; 128]).unwrap();
    assert!(fft.analyze(&[0.0; 65]).unwrap().iter().all(|&value| value == 0.0));
}

#[test]
fn planning_bounds_and_invalid_lengths_return_errors() {
    for length in [0, 1, 63, 65, MAX_AUDIO_FRAMES + 1, usize::MAX] {
        assert!(SpectrumFft::new(length).is_err());
    }
    let mut fft = SpectrumFft::new(64).unwrap();
    assert!(fft.analyze(&[1.0; 65]).is_err());
    assert!(compute_fft_mags(&vec![0.0; MAX_AUDIO_FRAMES + 1], |_, _| {}).is_err());
    assert!(fft.analyze(&[0.0; 64]).unwrap().iter().all(|&value| value == 0.0));
}

#[test]
fn analysis_and_warm_cache_lookup_do_not_allocate() {
    let samples: Vec<Vec<f64>> = (6..=16).map(|exponent| vec![0.25; 1 << exponent]).collect();
    for input in &samples {
        let mut fft = SpectrumFft::new(input.len()).unwrap();
        let allocations = allocation_counter::measure(|| {
            std::hint::black_box(fft.analyze(input).unwrap());
        });
        assert_eq!(allocations.count_total, 0, "first analysis at {}", input.len());
        compute_fft_mags(input, |_, _| {}).unwrap();
    }
    let allocations = allocation_counter::measure(|| {
        for input in samples.iter().rev().chain(samples.iter()) {
            compute_fft_mags(input, |_, magnitudes| {
                std::hint::black_box(magnitudes);
            })
            .unwrap();
        }
    });
    assert_eq!(allocations.count_total, 0, "alternating cached sizes");
}

#[test]
fn shared_plans_are_safe_for_parallel_frames() {
    std::thread::scope(|scope| {
        for worker in 0..8 {
            scope.spawn(move || {
                for count in [256, 4096, 256] {
                    let amplitude = (worker + 1) as f64 / 8.0;
                    let samples: Vec<f64> = (0..count).map(|index| amplitude * (2.0 * PI * 8.0 * index as f64 / count as f64).cos()).collect();
                    compute_fft_mags(&samples, |_, magnitudes| {
                        assert!((magnitudes[8] - amplitude).abs() < 0.01);
                    })
                    .unwrap();
                }
            });
        }
    });
}
