# Testing

```sh
cargo test --workspace     # everything
cargo xtask ci             # fmt, clippy -D warnings, tests, layers, assets, wasm
```

The `dev` profile keeps debug info to line tables (fast builds, readable backtraces). To debug
interactively — local variables, parameters, column breakpoints — build with the `dev-full`
profile: `cargo run --profile dev-full -p effectcraft -- --demo` (see `CONTRIBUTING.md`).

`cargo xtask ci` builds in the dev profile and runs the tests with `cargo nextest run` (plus
`cargo test --doc`) when `cargo-nextest` is installed, and with `cargo test` otherwise. It then
runs `missing_translations_report` with `--nocapture`, which prints the interface strings each
language has no translation for. Those strings show in English; the report always passes.

Each crate with several integration-test files has one test binary, `tests/it/main.rs`, whose
modules are the files in `tests/it/`. Run one file with `cargo test -p <crate> --test it <module>::`
or `cargo nextest run -p <crate> -E 'binary(it) and test(/^<module>::/)'`.

## What the tests cover

- **Engine commands** (`crates/engine/src/tests*.rs`): every command family, including undo and
  redo, and that every menu entry resolves to a registered command.
- **Rendering** (`crates/render`): pixel checks on small compositions, for blend modes, masks,
  mattes, time remapping, 3D projection, lighting and shadows. Fast paths (the layer cache, the
  affine compositor, blurs, motion blur accumulation) are checked against their reference
  implementations.
- **Effects** (`crates/effects`): each effect has a determinism test and at least one behaviour
  test (identity at neutral settings, known pixel results, or simulations giving the same frame
  whether you seek straight to it or play up to it). A registry-wide test checks that effects
  which read the clock are declared time-dependent. `crates/effects/tests/it/sim_golden.rs` pins
  exact pixel hashes of the simulation effects the GPU shares plans with (CC Rainfall … Card
  Wipe) at several settings, times, bit depths and resolutions, so any change to their CPU output
  fails (re-pin with `SIM_GOLDEN_PRINT=1` after an intended change; pinned for aarch64 macOS,
  with the hashes another macOS release's libm gives for unchanged code in `LIBM_VARIANTS`).
- **Export** (`crates/export/tests`): every format is encoded and decoded back, checking frame
  count, size and pixels. When `ffmpeg`/`ffprobe` are installed they are used as an outside
  check; they are never linked or shipped.
- **Automation** (`crates/automation`, `apps/effectcraft-cli/tests`): MCP protocol round trips and
  the command-line tool's JSON output.
- **Interface** (`crates/ui-egui`): headless egui_kittest tests for panels and dialogs.

## Looking at the interface without a window

```sh
cargo run -p effectcraft-ui-egui --example snapshot -- --out ui.png \
  --step '{"method":"engine.execute","params":{"command":"layer.select","params":{"layers":["#2"]}}}'
```

Each `--step` is a control-channel request; `{"method":"snap","params":{"path":"x.png"}}` writes an
intermediate image.

## Benchmarks

### Audio FFT migration (#1)

`generate3/audio.rs` uses cached f64 `craft-fft` complex plans for the eleven powers of two
from 64 through 65536. Forward transforms remain unscaled; the Hann window, zero padding,
`4/sample_count` magnitude scaling and band interpolation are unchanged. The shared backend's
inverse convention is `1/N`; the Opus test-only low-pass helper now consumes that normalized
inverse directly, without dividing a second time. Opus encoder MDCT is unchanged (#49).
Plans, backend scratch, complex samples and magnitudes are retained per size. Allocation tests
cover the first analysis after planning and warm cache lookups with alternating sizes. This
does not claim the entire effect is allocation-free: host audio, mark geometry and raster
buffers still allocate as before. Per-size locks serialize concurrent analysis of the same
size without locking rasterization, and cap the cache at eleven entries.

For a before/after comparison, run the ignored `gen_golden::audio_fft_migration_measurements`
integration test once before changing the implementation with `AUDIO_FFT_CAPTURE=1` and
`AUDIO_FFT_BASELINE=/absolute/path/audio-before.bin`, then again afterwards with the same
baseline path and without the capture flag. It compares every component of the 36 existing
Audio Spectrum/Waveform golden frames at two times, three parameter sets and three depth/scale
configurations, and asserts a maximum absolute delta of `1e-6`. The existing exact-hash test
remains enabled; do not use capture mode to replace its pins.

```sh
CARGO_TARGET_DIR="$PWD/target-1" CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  CARGO_PROFILE_RELEASE_DEBUG=0 RAYON_NUM_THREADS=1 \
  AUDIO_FFT_BASELINE="$PWD/target-1/audio-before.bin" \
  cargo test -p effectcraft-effects --release --test it audio_fft_migration_measurements -- --ignored --nocapture
```

Measured on aarch64 macOS, November 3, 2026: all 349,632 f32 components were bit-identical
(maximum delta zero; no golden updates). Warm 64×48 Spectrum frames, including host sampling
and CPU rasterization, took median 233.000 µs before versus 156.229–190.738 µs after at
N=8192 (170 ms audio), and 1560.740 µs before versus 935.306–1008.016 µs after at
N=65536 (1365 ms audio). The after ranges are two runs of the same implementation. Each
measurement uses 21 batches of 32 frames after eight warm-up frames, with one Rayon worker
and identical release settings. These are whole-effect measurements, not isolated FFT-kernel
speedups. Direct-DFT checks across every supported FFT size measured a maximum Hann-magnitude
delta of 9.68e-15; cached analysis and the first processing call after planning allocate zero times.

```sh
cargo run --release -p effectcraft-cli -- bench --n 10 --play 30
```

prints per-layer and per-effect timings for one frame, and playback timings with and without the
layer cache.

### Render benchmark (`cargo xtask bench`)

```sh
nice -n 10 cargo xtask bench --bin target/release/effectcraft-cli   # or no --bin: one `--release` build into target/bench
nice -n 10 cargo xtask bench --quick --compare before.jsonl        # 8 frames instead of 60, % change against an earlier run
cargo xtask bench --project F.ecproj --comp "Main"                 # any project's comp
```

measures memory, CPU and wall time of whole `effectcraft-cli` runs, so every optimisation has a
before and after number. It generates four deterministic scenarios into `target/bench/projects`
with engine commands (no committed media): `demo` (the built-in demo comp), `layers` (1080p,
100 solid, text and shape layers with Gaussian Blur, Glow, Levels, Curves, Drop Shadow and
keyframes), `precomps` (static and animated precomps nested three deep, each used several times)
and `footage` (a rendered PNG sequence and H.264 clip on several 1080p layers, plus a 4K still).
Each is opened (`open`), rendered at Full and Half (PNG sequence, deleted afterwards) and as one
frame. `/usr/bin/time -l` on macOS gives peak memory footprint, max RSS, user + sys CPU and
instructions retired; `/usr/bin/time -v` on Linux gives max RSS and CPU; elsewhere wall time only.

It prints a table and appends one JSON line per run to `target/bench/results.jsonl` (git rev,
date, load average, scenario, settings, metrics); `--compare OLD.jsonl` prints the % change against
the latest matching runs there, `--compare OLD --with NEW` compares two files without rendering.
`--repeat N` runs every scenario N times; with N > 1, one `"kind": "summary"` line per key follows
the runs. Its `metrics` are the medians of the successful runs (byte and instruction counts
rounded), `summary` holds `runs`, `failed` and median/p90/min/max per metric (p90 is the nearest
rank, so it equals the max for N < 10), and its other fields (`date`, `load`, ...) are those of the
key's last run; scripts that average runs should skip `kind: summary` lines. One failed run marks
the summary failed. Comparisons use the latest line per key, so a key is a median only if its
summary is the latest line (a later single run, or a batch that aborted before its summaries,
leaves a raw run latest). The table labels each side `median of N` or `1 run` and warns when they
differ; `--fail-over` refuses such keys.
The table notes the 1-minute load average: on a busy machine wall time is noise, so compare
peak memory and instructions retired, and rerun when the load differs a lot.

Results files allow blank lines; corrupt records or invalid comparison fields report the file
and line instead of being discarded. `--fail-over PCT` requires the same run keys on both sides,
successful latest runs, and at least one comparable resource metric per run. Failed latest runs,
missing coverage, empty results and wall-time-only measurements cannot pass the gate. Omit
`--fail-over` for an exploratory partial comparison; failed latest runs remain visible.
