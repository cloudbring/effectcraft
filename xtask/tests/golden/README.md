# Legacy xtask golden fixtures

Captured on 2026-11-03 from EffectCraft `7870b591` before deleting the legacy
`ico.rs`, `pdf_deps.rs`, and `version.rs` modules and inline `layers` checker.

- `layers.stdout`: output of `cargo xtask layers` on the workspace.
- `pdf-deps.stdout`: output of `cargo xtask pdf-deps` on the workspace.
- `version.stdout`: output of `cargo xtask version`.
- `version-set.stdout`, `version-after.toml`: output and rewritten root manifest
  for `version set 1.2.3-rc.1`.
- `ico.stdout`, `ico.hex`: output and packed bytes for `ico out.ico 16.png 32.png 48.png 256.png`
  on synthetic 16/32/48/256 px IHDR PNGs.
