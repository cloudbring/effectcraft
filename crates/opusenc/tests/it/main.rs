//! The crate's integration tests, one binary (each module is a former `tests/<name>.rs`).

mod common;
mod ffmpeg;
mod modes;

#[test]
fn shared_decoder_matches_filmcraft() {
    use effectcraft_opusenc::{Bandwidth, Mode, OpusEncoder};

    let signal = common::speech(8 * OpusEncoder::FRAME_SIZE, 49);
    let mut max_delta = 0.0f32;
    for channels in [1, 2] {
        for (mode, bandwidth, bitrate) in [
            (Mode::Celt, Bandwidth::Fullband, 64_000),
            (Mode::Hybrid, Bandwidth::Fullband, 32_000),
            (Mode::Hybrid, Bandwidth::SuperWideband, 24_000),
            (Mode::Silk, Bandwidth::Wideband, 16_000),
        ] {
            let mut encoder = OpusEncoder::with_mode(channels, bitrate * u32::from(channels), mode, bandwidth);
            let head = encoder.opus_head();
            let mut shared = craft_opus::Decoder::new(&head).unwrap();
            let mut reference = filmcraft_opus::Decoder::new(&head).unwrap();
            shared.set_trim_pre_skip(true);
            reference.set_trim_pre_skip(true);
            for frame in signal.as_chunks::<{ OpusEncoder::FRAME_SIZE }>().0 {
                let interleaved: Vec<f32> =
                    frame.iter().flat_map(|sample| (0..channels).map(move |channel| sample * (1.0 - 0.4 * f32::from(channel)))).collect();
                let packet = encoder.encode_float(&interleaved);
                let actual = shared.decode(Some(&packet)).unwrap();
                let expected = reference.decode(Some(&packet)).unwrap();
                assert_eq!(actual.len(), expected.len());
                for (actual_plane, reference_plane) in actual.iter().zip(&expected) {
                    assert_eq!(actual_plane.len(), reference_plane.len());
                    for (actual_sample, reference_sample) in actual_plane.iter().zip(reference_plane) {
                        let delta = (actual_sample - reference_sample).abs();
                        assert!(delta < 1e-5, "{mode:?}/{bandwidth:?} {channels} channels: {delta}");
                        max_delta = max_delta.max(delta);
                    }
                }
            }
        }
    }
    eprintln!("craft-opus vs FilmCraft maximum decoded PCM delta: {max_delta}");
}
