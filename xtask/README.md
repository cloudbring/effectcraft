# EffectCraft workspace tasks

Ticket #70 delegates reusable tooling to `craft-xtask` (`algo/integration`).
Only `xtask` depends on `craft-xtask`; runtime crates are unchanged.

| Task | Shared API | EffectCraft policy |
|---|---|---|
| `version` | `version::run`, `version::compute` | Default config (`Cargo.toml`, `target/` staging, offline-then-online lockfile refresh, permissive prerelease identifiers) |
| `ico` | `ico::run`, `ico::pack` | Default config (`root: "."`, square PNGs <= 256 px) |
| `layers` | `layers::run`, `layers::compute`, `layers::from_metadata` | EffectCraft L0–L6 table, `xtask` exempt, `ignore_dev: true`, 19 same-layer exceptions, UI/OS external boundary (`craft-ui`, `egui`, `eframe`, `egui-wgpu`, `winit`, `rfd`, `cpal`, `muda` only in L5–L6) |
| `pdf-deps` | `pdf_deps::compute`, `pdf_deps::validate` | Pinned `craft-pdf` (`exact_features = ["read"]`), `hayro-syntax` (`exact_features = ["std"]`), `hayro-jbig2` at `92d392cff132eaf53e13afff77f38b5c11b73aa0`; forbids active `hayro`, `hayro-interpret`, `hayro-cmap`, `krilla`; local one-line summary over `pdf_deps::Report` |

EffectCraft-specific tasks (`assets`, `wasm`, `web`, `audio-backend`, `bench`, and `ci`) remain local.
