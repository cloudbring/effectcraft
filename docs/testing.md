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

```sh
cargo run --release -p effectcraft-cli -- bench --n 10 --play 30
```

prints per-layer and per-effect timings for one frame, and playback timings with and without the
layer cache.

### Compositor scratch buffers (#112)

```sh
export CARGO_TARGET_DIR="$PWD/target-112"
export RUSTC="$(rustup which --toolchain stable rustc)"
export RUSTDOC="$(rustup which --toolchain stable rustdoc)"
export PATH="$(dirname "$RUSTC"):$PATH"
rustup run stable cargo test --locked --release -p effectcraft-render \
  compose_buffers_1080p -- --ignored --nocapture
```

The ignored benchmark renders 60 animated, uncached 1080p frames after one warmup frame,
with eight solid layers (seven preserving transparency). It measures both full-canvas and
480×270 layer sources. One Rayon worker runs both rendering and the thread-local
`allocation-counter` measurement; this includes the returned image and evaluator metadata,
not just scratch storage. `allocation-counter` is a dev-only MIT/Apache-2.0 dependency.

Measured on November 3, 2026, Apple M6, release profile, Cargo 1.99.0 via rustup and
Homebrew rustc 1.99.0 (`b940084d7`); baseline production code:
`7870b5916ae0749d497d880989683bf2946d6c8b`. Both sides used the same compiler. The commands
above explicitly pin rustup's compiler too, as do the final quality checks: on this machine
`rustup run stable cargo` alone still found Homebrew rustc because the rustup rustc shim
was absent from `~/.cargo/bin`.

| Layer size | Before ms/frame | After ms/frame | Before allocations/frame | After allocations/frame | Before allocated bytes/frame | After allocated bytes/frame |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1920×1080 | 69.138 | 75.795 | 595.6 | 586.6 | 531,047,326 | 33,383,702 |
| 480×270 | 15.575 | 6.367 | 371.6 | 362.6 | 282,037,726 | 33,206,102 |

These are single-run wall times on a shared machine, not a controlled throughput claim.
Earlier after runs were 78.919/5.926 and 195.972/21.651 ms/frame respectively while other
Rust builds were running; allocation counts/bytes were identical in every after run.
Full-canvas time has not improved in these measurements. Allocated bytes drop by 93.7%
and 88.2%; the remaining approximately 33 MB/frame is primarily the owned output image.

The renderer owns a size-keyed scratch pool shared with its nested renderers. Buffers are
returned after each layer/pass, fully initialized before reuse, and retained across frames
up to 16 slots/192 MiB. Only uniquely owned source buffers are recycled; cached/shared
sources remain untouched. Affine isolation, matte application and preserve-alpha blending
use the warp's affected rectangle (the union for styled passes). Stencils, motion-blurred
placement and unknown bounds retain full-canvas processing. Dissolve keeps canvas-global
noise coordinates. Clearing scratch still covers its whole allocation.

Correctness verification captured all 859 output frames from the baseline render library
tests as dimensions plus raw little-endian f32 bits, then compared every byte against the
pooled implementation; all matched. This temporary capture instrumentation was removed,
and no golden images were changed. Permanent tests compare bounded/full passes for every
blend mode and matte kind, moving/resized pooled frames against fresh renderers, and pool
clearing/retention. Re-compositing the same prepared frame allocates **zero scratch buffers**;
Rayon still allocates 24 small task records (1,472 bytes) for eight layers. A separate full
renderer test bounds allocations to the returned image plus evaluator metadata. Neither
test claims that the whole renderer is allocation-free.

Final validation with explicitly pinned rustup stable and `CARGO_TARGET_DIR=target-112`:
render/raster tests passed (385 passed, 0 failed, 2 ignored); crate and workspace
`clippy --all-targets -- -D warnings`, formatting and all `cargo xtask ci` stages passed
(5,011 tests passed, 0 failed, 33 ignored; 40 attributed assets; 33 wasm crates).
The disk-cache tests needed `TMPDIR` on a disposable 4 GiB APFS sparse volume under
`target-112/target/`: host free space was below their existing cache reservation threshold.
The volume was detached and all `target-112` artifacts removed after validation.
Existing wasm UI unused-variable/dead-code warnings remain unrelated to this change.

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
