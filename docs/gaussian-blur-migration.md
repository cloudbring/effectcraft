# Variance-exact Gaussian migration (#2)

Measured on 2026-11-03 in `algo/2-effectcraft-craft-blur`, against its original main
checkout. The shared dependency is `cloudbring/craft-libs`, branch `algo/2-craft-blur`,
locked to `75bb40cf0c06b7e046874f389376acbfd86915ab`. Repoint it to
`storytold/craft-libs` after the shared implementation merges.

## Kernel and integration contract

`effectcraft-raster` delegates its premultiplied RGBA Gaussian to `craft-blur`.
`Quality::Recursive` uses the shared calibrated sampled kernel below sigma 8 and
the variance-calibrated Young–van Vliet recurrence with Triggs–Sdika boundaries
above that threshold. This is the actual contract at the locked commit, including
the sampled guard band; the issue's earlier sigma-2.5 description is incomplete.
`gaussian_blur_exact` delegates to shared `Quality::Exact`.

Repeat Edge selects `Edge::Clamp`; otherwise the image has transparent padding.
Channels are filtered independently without converting out of premultiplied RGBA.
Rayon workers own complete horizontal rows or vertical columns: row batches are
64 rows and column stripes are 64 pixels wide. Scratch is reused within each job;
no recursive state is restarted halfway along a line. Tests compare the parallel
adapter bit-for-bit with the serial shared filter, including HDR and odd widths.

`try_gaussian_blur` validates dimensions, propagates shared errors and accepts
`BlurCancel`. A cancelled render discards its partial copy and leaves the input
unchanged. Existing infallible effect entrypoints log failures and preserve the
input; hosts needing explicit failure/cancellation reporting can use the fallible
entrypoint. Zero, negative, NaN and tiny sigma retain the existing identity behavior.

Gaussian Blur and its obsolete variant, plus Gaussian-using auxiliary effects,
use the shared implementation. Glow explicitly uses shared Exact in modern mode:
the recursive impulse shape otherwise misses its existing After Effects oracle.
Fast Box Blur and independent scalar-plane box approximations intentionally remain
box filters. Directional, radial and lens algorithms are unchanged. Text animator
blur also retains its prior box kernel explicitly, outside the effects migration.

## Project compatibility and GPU dispatch

Schema 3 records `settings.legacy_gaussian_blur`. New projects default to false;
files missing the field default to true. Use
`file.projectSettings {"legacyGaussianBlur": true}` to opt into compatibility,
with undo support and normal project saving. Gaussian effects retain their old
sampled-below-3/three-box crossover; auxiliary Gaussians retain the old three-box
shape. Compatibility preserves the kernel choice and variance, not bit-identical
old f32 running sums. The shared Exact kernel is variance-calibrated too.

Both processed-layer cache keys and deferred GPU effect-chain keys include the
flag. Modern Gaussian effects and affected auxiliary effects fall back to CPU
until the GPU implements the same kernel; legacy mode keeps the existing GPU
path. Explicit Fast Box Blur remains GPU-supported.

EffectCraft owns the schema-3 validation gate because the pinned
`craft-ecproj-format` only accepts schema 2. The shared crate still supplies the
file envelope and saved-view/version metadata. Schema-1 effect-space migration,
schema-3 save/readback, missing-flag compatibility and undo are regression-tested.

## Impulse variance

The original raster Gaussian used three integer-width box passes. For these
integer sigmas its relative variance deficit is `1/(3 sigma)`:

| Sigma | Original variance | Original error | Shared variance |
| ---: | ---: | ---: | ---: |
| 1 | 0.666666667 | -33.333333% | 1.000000007 |
| 4 | 14.666671579 | -8.333303% | 16.000000195 |
| 16 | 250.666721503 | -2.083312% | 256.000000258 |
| 64 | 4074.667575998 | -0.520811% | 4095.999999101 |

The original measurement integrates only the known finite support (within
plus/minus three sigma), excluding the old f32 running-sum residual floor outside
that support. The new measurement integrates the full impulse image, extending
32 sigma in both directions. Horizontal and vertical results agree; maximum
relative error is about `1.2e-8`, well within the required 0.5%. The main Gaussian
effect already used a sampled kernel below sigma 3, so the original sigma-1 box
deficit describes the raster entrypoint, not that effect's old small-sigma path.

## Golden updates

Following `docs/testing.md`, all 1,085 frames from the four effects integration
golden suites were captured before and after as little-endian RGBA f32. Exactly
eight frames changed; all other frames were byte-identical. Each changed pin and
its maximum absolute channel delta is listed here:

| Golden | Maximum delta |
| --- | ---: |
| `ec.sim.caustics/1/8bpc/t0.5` | 0.090196073055 |
| `ec.sim.caustics/1/8bpc/t0.8` | 0.090196073055 |
| `ec.sim.caustics/1/8bpc/t1.5` | 0.090196073055 |
| `ec.sim.caustics/1/8bpc/t2.2` | 0.090196073055 |
| `ec.sim.caustics/1/32bpc/t0.5` | 0.102140188217 |
| `ec.sim.caustics/1/32bpc/t0.8` | 0.102140188217 |
| `ec.sim.caustics/1/32bpc/t1.5` | 0.102140188217 |
| `ec.sim.caustics/1/32bpc/t2.2` | 0.102140188217 |

The integer-depth maximum is 23/255. No tolerance was loosened, and the already
accepted CC Mr. Mercury libm variant was not re-pinned. The Gaussian effect itself
has direct variance, inactive-axis and compatibility regression tests rather than
an existing image-golden suite.

## 1080p Gaussian effect benchmark

Apple M6, native rustup Rust 1.99.0, release profile, `RAYON_NUM_THREADS=12`.
The example `crates/effects/examples/gaussian_bench.rs` renders the actual effect
on deterministic premultiplied 1920-by-1080 RGBA, with Repeat Edge enabled and
blurriness `2 sigma`. Input cloning is excluded; output allocation and filtering
are included. Each invocation warms up twice and takes the median of seven
renders. The table is the median of three final interleaved original/new pairs;
no compilation ran concurrently with these measurements.

| Sigma | Before medians (ms) | After medians (ms) | Before median (ms) | After median (ms) |
| ---: | --- | --- | ---: | ---: |
| 2 | 7.773, 7.077, 7.189 | 8.343, 8.069, 8.212 | 7.189 | 8.212 |
| 16 | 4.051, 4.000, 4.179 | 4.326, 4.333, 4.382 | 4.051 | 4.333 |
| 64 | 4.434, 4.304, 5.305 | 4.356, 4.320, 4.155 | 4.434 | 4.320 |

This run is 14.2% slower at sigma 2, 7.0% slower at sigma 16 and 2.6% faster at
sigma 64. The machine was shared and heavily loaded (load averages around 40.7,
35.7, 35.1); earlier pairs varied substantially, reaching 50 ms. These are observed
timings, not a claim of a stable speedup. The change fixes variance, not every
multichannel performance cost.

Reproduce the new path with:

```sh
export PATH="$(dirname "$(rustup which cargo)"):$PATH"
export CARGO_TARGET_DIR=/Users/emm/ArtCraft/worktrees/effectcraft-2-blur/target-2
RAYON_NUM_THREADS=12 cargo run -p effectcraft-effects --release --example gaussian_bench
```

## Validation scope

The required raster/effects tests, project tests, render tests, compatibility
command and schema-migration tests, GPU cache-key and catalogue-dispatch tests,
all-target clippy for the six touched crates, formatting and `cargo xtask layers`
pass using the rustup toolchain and dedicated `target-2` directory:

- `cargo test -p effectcraft-raster -p effectcraft-effects`: 603 passed, zero
  failed, one pre-existing ignored test (551 effects unit, six effects integration,
  46 raster unit tests).
- Supplementary project/render/engine/GPU checks: 389 passed, zero failed, one
  pre-existing ignored render test (49 project, 335 render, two engine and three
  GPU tests, including the GPU effects-versus-CPU test).
- `cargo clippy -p effectcraft-raster -p effectcraft-effects -p effectcraft-project
  -p effectcraft-render -p effectcraft-engine -p effectcraft-gpu --all-targets --
  -D warnings`, `cargo fmt --all -- --check`, and `cargo xtask layers`: pass.

Full
workspace CI, assets and wasm gates are deliberately skipped for this scoped
migration. Temporary snapshots, logs, benchmark binaries and `target-2` are removed
after validation; only code, golden pins and this report are committed.
