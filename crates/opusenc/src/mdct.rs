//! CELT low-overlap analysis adapter (RFC 6716 §4.3.7).
//!
//! Pad the N+120 sample block to 2N, compensate craft-fft's sine window with the
//! CELT window, then scale its unnormalised coefficients by 2/N. The FFT plan,
//! compensation gains and scratch are allocated only at construction.

use craft_fft::{Error, TdacWindow};

use craft_opus::celt_tables::OVERLAP;

pub struct Mdct {
    transform: craft_fft::Mdct<f32>,
    input: Vec<f32>,
    gains: Vec<f32>,
    #[cfg(test)]
    window: [f32; OVERLAP],
}

impl Mdct {
    pub fn new(coefficients: usize) -> Result<Self, Error> {
        if !matches!(coefficients, 120 | 240 | 480 | 960) {
            return Err(Error::InvalidSize);
        }
        let transform = craft_fft::Mdct::new(coefficients, TdacWindow::Sine)?;
        let window: [f32; OVERLAP] = std::array::from_fn(|index| {
            let phase = std::f64::consts::FRAC_PI_2 * (index as f64 + 0.5) / OVERLAP as f64;
            let sine = phase.sin();
            (std::f64::consts::FRAC_PI_2 * sine * sine).sin() as f32
        });
        let pad = (coefficients - OVERLAP) / 2;
        let gains = transform
            .window()
            .iter()
            .skip(pad)
            .take(coefficients + OVERLAP)
            .enumerate()
            .map(|(index, sine)| {
                let celt = if index < OVERLAP {
                    window[index]
                } else if index >= coefficients {
                    window[OVERLAP - 1 - (index - coefficients)]
                } else {
                    1.0
                };
                celt / sine
            })
            .collect();
        Ok(Self {
            transform,
            input: vec![0.0; 2 * coefficients],
            gains,
            #[cfg(test)]
            window,
        })
    }

    /// Preserves the block; invalid lengths leave the output untouched.
    pub fn forward(&mut self, block: &[f32], output: &mut [f32]) -> Result<(), Error> {
        let coefficients = self.transform.coefficients_len();
        if block.len() != coefficients + OVERLAP {
            return Err(Error::Length { expected: coefficients + OVERLAP, actual: block.len() });
        }
        if output.len() != coefficients {
            return Err(Error::Length { expected: coefficients, actual: output.len() });
        }
        let pad = (coefficients - OVERLAP) / 2;
        for ((sample, value), gain) in self.input.iter_mut().skip(pad).zip(block).zip(&self.gains) {
            *sample = value * gain;
        }
        self.transform.process_into(&self.input, output)?;
        for value in output {
            *value *= 2.0 / coefficients as f32;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_fft::{Complex, ComplexFft, Direction};

    fn direct(block: &[f32], coefficients: usize, window: &[f32; OVERLAP]) -> Vec<f64> {
        let pad = (coefficients - OVERLAP) / 2;
        let mut input = vec![0.0f64; 2 * coefficients];
        for (index, &value) in block.iter().enumerate() {
            let gain = if index < OVERLAP {
                window[index]
            } else if index >= coefficients {
                window[OVERLAP - 1 - (index - coefficients)]
            } else {
                1.0
            };
            input[pad + index] = (value * gain) as f64;
        }
        (0..coefficients)
            .map(|bin| {
                input
                    .iter()
                    .enumerate()
                    .map(|(time, sample)| {
                        sample * (std::f64::consts::PI / coefficients as f64 * (time as f64 + 0.5 + coefficients as f64 / 2.0) * (bin as f64 + 0.5)).cos()
                    })
                    .sum::<f64>()
                    * 2.0
                    / coefficients as f64
            })
            .collect()
    }

    #[test]
    fn craft_fft_matches_dft() {
        for length in [2usize, 3, 4, 5, 6, 8, 12, 15, 16, 20, 24, 30, 48, 60, 120, 240, 480] {
            let mut fft = ComplexFft::new(length).unwrap();
            let input: Vec<_> =
                (0..length).map(|index| Complex::new(((index * 37 % 101) as f32 - 50.0) / 7.0, ((index * 53 % 89) as f32 - 44.0) / 9.0)).collect();
            let mut output = input.clone();
            fft.process_in_place(Direction::Forward, &mut output).unwrap();
            for (bin, actual) in output.iter().enumerate() {
                let mut expected = Complex::new(0.0f64, 0.0);
                for (index, value) in input.iter().enumerate() {
                    let phase = -std::f64::consts::TAU * ((index * bin) % length) as f64 / length as f64;
                    expected += Complex::new(value.re as f64, value.im as f64) * Complex::new(phase.cos(), phase.sin());
                }
                assert!((actual.re as f64 - expected.re).abs() < 2e-3 && (actual.im as f64 - expected.im).abs() < 2e-3, "length={length} bin={bin}");
            }
        }
    }

    #[test]
    fn matches_direct_formula() {
        for coefficients in [120usize, 240, 480, 960] {
            let block: Vec<f32> = (0..coefficients + OVERLAP).map(|index| ((index * 37 % 101) as f32 - 50.0) / 7.0).collect();
            let mut transform = Mdct::new(coefficients).unwrap();
            let mut output = vec![0.0; coefficients];
            transform.forward(&block, &mut output).unwrap();
            let expected = direct(&block, coefficients, &transform.window);
            for (bin, (actual, reference)) in output.iter().zip(expected).enumerate() {
                assert!((*actual as f64 - reference).abs() < 1e-2, "coefficients={coefficients} bin={bin}: {actual} vs {reference}");
            }
        }
    }

    /// FilmCraft's CELT inverse adapter: retain the middle N samples, undo the
    /// sine window and 2/N scale, then rotate the 120-sample low-overlap tail.
    #[test]
    fn perfect_reconstruction_through_decoder_imdct() {
        for coefficients in [120usize, 240, 480, 960] {
            let frames = 5;
            let signal: Vec<f32> =
                (0..coefficients * frames).map(|index| (index as f32 * 0.013).sin() * 1000.0 + ((index * 7919) % 211) as f32 - 105.0).collect();
            let mut forward = Mdct::new(coefficients).unwrap();
            let mut inverse = craft_fft::Imdct::new(coefficients, TdacWindow::Sine).unwrap();
            let half = coefficients / 2;
            let gains: Vec<f32> = inverse.window()[half..3 * half].iter().map(|window| half as f32 / window).collect();
            let mut inverse_output = vec![0.0; 2 * coefficients];
            let mut history = vec![0.0; OVERLAP];
            let mut memory = vec![0.0; coefficients + OVERLAP];
            let mut decoded = Vec::new();
            for frame in signal.chunks_exact(coefficients) {
                let mut block = history.clone();
                block.extend_from_slice(frame);
                history.copy_from_slice(&block[coefficients..]);
                let mut spectrum = vec![0.0; coefficients];
                forward.forward(&block, &mut spectrum).unwrap();
                memory.copy_within(coefficients..coefficients + OVERLAP / 2, 0);
                inverse.process_into(&spectrum, &mut inverse_output).unwrap();
                for ((sample, value), gain) in memory[OVERLAP / 2..OVERLAP / 2 + coefficients].iter_mut().zip(&inverse_output[half..3 * half]).zip(&gains) {
                    *sample = value * gain;
                }
                for index in 0..OVERLAP / 2 {
                    let left = memory[index];
                    let right = memory[OVERLAP - 1 - index];
                    let rising = forward.window[index];
                    let falling = forward.window[OVERLAP - 1 - index];
                    memory[index] = left * falling - right * rising;
                    memory[OVERLAP - 1 - index] = left * rising + right * falling;
                }
                decoded.extend_from_slice(&memory[..coefficients]);
            }
            let mut error = 0.0f64;
            let mut energy = 0.0f64;
            for (actual, reference) in decoded[OVERLAP..].iter().zip(&signal) {
                error += (*actual as f64 - *reference as f64).powi(2);
                energy += (*reference as f64).powi(2);
            }
            let snr = 10.0 * (energy / error).log10();
            assert!(snr > 80.0, "coefficients={coefficients}: reconstruction SNR {snr:.1} dB");
        }
    }

    #[test]
    fn processing_including_first_call_does_not_allocate() {
        for coefficients in [120usize, 240, 480, 960] {
            let mut transform = Mdct::new(coefficients).unwrap();
            let input = vec![0.25; coefficients + OVERLAP];
            let mut output = vec![0.0; coefficients];
            let allocations = allocation_counter::measure(|| {
                for _ in 0..16 {
                    transform.forward(&input, &mut output).unwrap();
                }
            });
            assert_eq!(allocations.count_total, 0);
        }
    }

    #[test]
    fn invalid_arguments_leave_output_untouched() {
        for coefficients in [0, 119, 121, 239, 961, usize::MAX] {
            assert!(matches!(Mdct::new(coefficients), Err(Error::InvalidSize)));
        }
        let mut transform = Mdct::new(960).unwrap();
        for (input_length, output_length) in [(1079, 960), (1081, 960), (1080, 959), (1080, 961)] {
            let mut output = vec![7.0; output_length];
            assert!(transform.forward(&vec![0.0; input_length], &mut output).is_err());
            assert!(output.iter().all(|value| *value == 7.0));
        }
    }
}
