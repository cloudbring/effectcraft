# effectcraft-opusenc

Clean-room, pure-Rust Opus encoder for EffectCraft's WebM export audio (layer L0, `craft-fft` transforms,
no `unsafe`, builds for `wasm32-unknown-unknown`).

All three Opus modes, 20 ms packets (one frame per packet), 48 kHz float input, mono or stereo:

- **CELT** (fullband): MDCT transform coding, constant packet size.
- **SILK** (narrowband 8 kHz, mediumband 12 kHz, wideband 16 kHz internal rate): linear
  prediction with long-term (pitch) prediction; packet size varies around the target bitrate.
- **Hybrid** (super-wideband, fullband): SILK wideband codes 0–8 kHz and CELT codes bands 17 and
  up (8 kHz and above) in the same range-coded frame; constant packet size.

```rust
use effectcraft_opusenc::{Application, OpusEncoder};
let mut enc = OpusEncoder::with_application(2, 48_000, Application::Voip);
let packet = enc.encode_float(&vec![0.0; OpusEncoder::FRAME_SIZE * 2]);
// enc.mode(), enc.bandwidth(), enc.pre_skip(), enc.opus_head() ...
```

`OpusEncoder::new(channels, bitrate)` is `with_application(.., Application::Audio)`.
`OpusEncoder::with_mode(channels, bitrate, mode, bandwidth)` forces a mode/bandwidth pair
(SILK: NB/MB/WB; hybrid: SWB/FB; CELT: FB).

## Mode table

Chosen once per stream (constant-bitrate export, no in-stream switching) from the bitrate per
channel (bitrate / channels; the total is clamped to 6 kb/s per channel … 510 kb/s):

| per-channel bitrate | `Application::Voip`     | `Application::Audio` (default) |
|---------------------|-------------------------|--------------------------------|
| < 10 kb/s           | SILK narrowband (8 kHz) | SILK wideband                  |
| 10–12 kb/s          | SILK mediumband (12 kHz)| SILK wideband                  |
| 12–16 kb/s          | SILK wideband (16 kHz)  | SILK wideband                  |
| 16–18 kb/s          | SILK wideband           | hybrid fullband                |
| 18–28 kb/s          | hybrid super-wideband   | hybrid fullband                |
| 28–36 kb/s          | hybrid fullband         | hybrid fullband                |
| 36–40 kb/s          | hybrid fullband         | CELT fullband                  |
| ≥ 40 kb/s           | CELT fullband           | CELT fullband                  |

TOC configurations (RFC 6716 §3.1, Table 2): SILK NB/MB/WB 20 ms = 1/5/9, hybrid SWB/FB 20 ms
= 13/15, CELT FB 20 ms = 31; the stereo bit is set for two channels; frame-count code 0.

`pre_skip()` is 120 samples in every mode: the CELT MDCT overlap, and for SILK/hybrid the
decimator delay (83 samples at 48 kHz, 88 for narrowband) plus the decoder's SILK delay (one
internal-rate sample and the resampler delay of RFC 6716 Table 54). The SILK and CELT halves of a
hybrid frame are therefore aligned without extra buffering.

## What is implemented

**SILK** (RFC 6716 §4.2, encoder direction; own analysis and search design):

- 48 kHz → 8/12/16 kHz decimation with a linear-phase Kaiser-windowed-sinc FIR.
- Per frame: energy VAD (inactive / unvoiced / voiced), open-loop pitch search (normalised
  correlation with octave checks, joint lag + contour codebook choice, §4.2.7.6.1), windowed
  autocorrelation LPC (Levinson, lag window, bandwidth expansion), LPC → NLSF by root search,
  NLSF quantisation (stage-1 preselection by weighted distance, backwards-predictive stage-2
  scalar quantiser reproducing the decoder's dequantisation exactly, rate-distortion choice),
  LTP codebook search over the three periodicity codebooks, independent + delta gain coding.
- Analysis-by-synthesis excitation: every sample is chosen by running the decoder's exact
  fixed-point reconstruction (LTP from the decoder's own past output, LPC synthesis with the Q12
  coefficients of the *dequantised* NLSFs, gain rescaling between subframes, quantisation
  offsets, the LCG sign sequence), so encoder and decoder never drift. A noise-feedback loop
  shapes the coding noise by `1/A(z/γ)` (γ 0.75–0.9, coarser quantisation → more shaping), and a
  small rate penalty biases pulses towards zero.
- Excitation coding: rate level, per-block pulse counts with LSB escapes, shell coding, LSBs and
  signs (§4.2.7.8).
- Rate control: per-frame search over a gain offset (integer bracket, then bisection) to the
  frame budget; SILK-only streams carry a bit reservoir across frames (packet sizes vary).
- Stereo: mid/side with the decoder's stereo prediction (§4.2.7.1, §4.2.8; ridge-regularised
  least-squares weights, quantised to the decoder grid, interpolation mirrored), mid-only frames
  when the predicted side is negligible, side gains floored at the mid gains.
- Not implemented: LBRR (in-band FEC), NLSF interpolation, 10/40/60 ms frames, DTX.

**Hybrid** (§4.3 with start band 17): the SILK frame is written first, then the redundancy flag
(always 0) when there is room, then a CELT frame coding bands 17–18 (SWB) or 17–20 (FB) in the
same range coder with the remaining packet bytes (no silence flag or post-filter in hybrid
frames, decoder band-energy reset for uncoded bands mirrored). SILK gets 72 % (SWB) / 62 % (FB)
of the per-channel bitrate, at most the bitrate minus 6 kb/s, and never more than leaves CELT a
fifth of the packet (at least 80 bits).

**CELT**: shared forward MDCT with the low-overlap window, band energies with inter/intra
coarse prediction, fine and final energy bits, the decoder's exact bit allocation, spreading,
recursive band splitting, greedy PVQ search. Not implemented: transient frames, TF changes,
pitch post-filter, intensity stereo, dynamic allocation, VBR. All-zero CELT frames become 3-byte
silence packets.

## Measured (tests in `tests/`, FilmCraft decoder)

Synthetic speech (glottal pulses 100–200 Hz through three moving formants, syllable envelopes,
pauses) and music (three-note harmonic chords); waveform SNR against the input band-limited to
the mode's bandwidth (full band for hybrid/CELT), residual delay 0 (±1 for narrowband) after the
pre-skip:

| mode / bandwidth   | bitrate target → actual | speech SNR | music SNR |
|--------------------|-------------------------|------------|-----------|
| SILK NB            | 8 → 7.5 kb/s            | 15.9 dB    | 6.9 dB (9 kb/s) |
| SILK MB            | 11 → 10.1 kb/s          | 21.8 dB    | 7.9 dB    |
| SILK WB            | 12 → 10.9 kb/s          | 21.5 dB    | 8.1 dB    |
| SILK WB            | 16 → 14.3 kb/s          | 28.1 dB    | 12.1 dB   |
| SILK WB            | 24 → 21.1 kb/s          | 37.0 dB    | 19.7 dB   |
| hybrid SWB         | 24 kb/s (CBR)           | 28.7 dB    | 13.0 dB   |
| hybrid FB          | 32 kb/s (CBR)           | 32.1 dB    | 15.6 dB   |
| CELT FB            | 12 kb/s (CBR)           | 13.0 dB    | —         |

At 12 kb/s mono, SILK wideband beats the CELT path on speech by ~8 dB (21.0 vs 13.0 dB
full-band SNR). SILK bitrates land within −13 % … 0 % of the target (speech spends little in
pauses), hybrid is exact. Stereo SILK WB at 32 kb/s: 41 dB dual-mono, 40/37 dB panned, 22/19 dB
for two independent speech signals.

ffmpeg (external oracle, Ogg Opus written by the test) decodes every mode with both its native
`opus` decoder and `libopus` without errors, honouring pre-skip and end trimming, with the same
SNRs (libopus residual delay 0, ffmpeg's native decoder 0–4 samples). The only message tolerated
is ffmpeg's raw-PCM muxer timestamp complaint that its native decoder also prints for
narrowband files encoded by libopus itself.

## Specifications

- RFC 6716, *Definition of the Opus Audio Codec* (September 2012), with the RFC 8251 updates
  (October 2017) as implemented by the decoder we round-trip against.
- RFC 7845, *Ogg Encapsulation for the Opus Audio Codec* (April 2016), §5.1 `OpusHead` and
  §4.2 pre-skip.

## Shared transforms and CELT table API (#49)

`craft-fft` and the test decoder `craft-opus` use `cloudbring/craft-libs` branch
`algo/integration`, locked at `249cbcc191bbf14558577677b239a45a7e724c72`. Repoint both
dependencies to the upstream release/revision when that branch lands.

The analysis adapter pads N+120 samples at offset (N−120)/2 in a 2N block, multiplies
by the CELT/sine window ratio, and scales `craft_fft::Mdct` output by 2/N. Plans,
gains and scratch are cached; the first and subsequent transforms allocate nothing.
Tests use `craft_fft::Imdct` with FilmCraft a29f2893's middle-half extraction,
N/2 inverse scaling and low-overlap TDAC rotation. All four CELT sizes are tested.
No craft-fft API addition is needed. CELT framing/windowing belongs in the codec;
if shared later, put the adapter in craft-opus, not the codec-neutral craft-fft.

**No CELT table is currently publicly importable.** `craft-opus::celt` and its
submodules are `pub(crate)`. Existing definitions remain unchanged, not newly
copied. A standalone Rust equality check verified every integer entry and every
float's `to_bits()` in the following existing definitions against that revision:

- Proposed public `craft_opus::celt_tables` re-exports: `NB_EBANDS`, `EBANDS`,
  `OVERLAP`, `MAX_LM`, `PREEMPH`, `ALLOC`, `E_MEANS`, `PRED_COEF`, `BETA_COEF`,
  `BETA_INTRA`, `E_PROB_MODEL`, `SMALL_ENERGY_ICDF`, `LOG2_FRAC_TABLE`,
  `TF_SELECT_TABLE`, `SPREAD_ICDF`, `TRIM_ICDF` (16 definitions, 11 arrays).
- Also move/re-export the byte-identical function-local `EXP2_TABLE8: [i32; 8]`
  and `SPREAD_FACTOR: [usize; 3]` from `celt::bands` into that public table API.
- Identical rate constants still private to the codec: `MAX_PSEUDO`,
  `LOG_MAX_PSEUDO`, `MAX_FINE_BITS`, `FINE_OFFSET`, `QTHETA_OFFSET`,
  `QTHETA_OFFSET_TWOPHASE`, `ALLOC_STEPS` (all `i32`); expose them there too.

After these API additions, replace the local definitions with imports and move
craft-opus from dev-dependencies to dependencies. Generated PVQ/pulse caches and
SILK codebooks are outside this constant-table migration.

Validation on 2026-11-03, rustup stable rustc 1.99.0, aarch64-apple-darwin:
the existing suites retained their SNR/spectral tolerances. Of 7,179 packets captured
from their shared encode helpers, 505 changed; maximum decoded PCM difference from
the old encoder through the same FilmCraft decoder was 0.034308255 (stereo 128 kb/s).
This is quantizer/bitstream divergence, not just inverse-transform rounding.
The checkout has 10 export `opus_roundtrip` tests (not 11) and originally 12 opusenc
integration tests; the added shared-decoder comparison brings the latter to 13.
The new comparison covers mono/stereo CELT, hybrid SWB/FB and SILK WB; decoding
the same packets with craft-opus versus FilmCraft differs by at most 1.1920929e-7.
Package tests: 151 passed, 0 failed, 1 pre-existing ignored VP9 benchmark.
Package clippy (`--all-targets -- -D warnings`), fmt and `cargo xtask layers` pass.
Workspace-wide CI, tests/clippy, assets and wasm gates were intentionally skipped.

Reproduce the benchmark with `cargo run --release -p effectcraft-opusenc --example
encode_benchmark`: 60 s stereo 48 kHz, 128 kb/s, deterministic two-tone channels,
pre-generated PCM, one-second warm-up, encoding only (not decoding or construction).
Nine trials each, alternating the original and migrated release executables:
median **0.134372 s before / 0.117525 s after**; each emitted 960,000 packet bytes.
Observed ranges were 0.126179–0.172061 s before and 0.115725–0.123561 s after.
These are warm local timings, not an end-to-end export or quality benchmark.

## Provenance

Implemented from the RFC text. Constant tables (CELT band edges, `eMeans`, the coarse-energy
Laplace model, the allocation table, ICDFs; all SILK probability tables and codebooks: NLSF
stage-1/2 codebooks and prediction weights, gain, pitch, contour, LTP and pulse ICDFs, stereo
weights) and the decoder-side normative arithmetic the encoder must reproduce bit-exactly (CELT
allocation; SILK NLSF dequantisation and stabilisation, NLSF → LPC conversion, `log2lin`, the
fixed-point division/inverse helpers and the LPC analysis filter) are mirrored from FilmCraft's
first-party, spec-derived `filmcraft-opus` decoder (MIT OR Apache-2.0), which is also the
round-trip oracle in `tests/` and `crates/export/tests/it/opus_roundtrip.rs` (ffmpeg is a second,
external oracle when installed). All encoder-side analysis and search (pitch, LPC, NLSF and
LTP search, noise feedback, rate control, stereo prediction, the decimator) is our own design.
No third-party encoder or decoder source (libopus or otherwise) was consulted.
