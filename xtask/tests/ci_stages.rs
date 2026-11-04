//! #188: `cargo xtask ci` runs the same stages, in the same order, with the same literal cargo
//! arguments, as before the `craft-xtask` swap (#70).
//!
//! OLD is storytold/effectcraft @ 7870b5916ae0749d497d880989683bf2946d6c8b (merge-base of
//! cloudbring/effectcraft `algo/70-use-craft-xtask` with `origin/main`); `golden/ci-stages.txt` is the
//! stage table of its `fn ci()` reduced to `name | literal args` lines.
//!
//! The effectcraft xtask spawns cargo through the compile-time `env!("CARGO")`, so unlike photocraft and
//! vectorcraft the run cannot be intercepted with a PATH shim; this compares the stage table in the source.
//! Observed once by running `xtask ci` for real in both trees (not reproducible here, as it needs the
//! wasm32 target): the `$ ` command lines are identical (fmt, clippy, tests, the i18n report test binary,
//! doctests, wasm check), both fail at `wasm` for the same reason (target not installed), and the
//! `wasm` package list has the same 33 `-p` packages, sorted by name in NEW and in workspace order in OLD.

fn stage_table(src: &str) -> String {
    let block = src.split("let stages").nth(1).expect("`let stages` in xtask/src/main.rs");
    let block = block.split("];").next().unwrap();
    let mut out = String::new();
    for line in block.lines().filter(|l| l.trim_start().starts_with("(\"")) {
        let name = line.trim_start().strip_prefix("(\"").unwrap().split('"').next().unwrap();
        let args = line.split("args([").nth(1).and_then(|a| a.split("])").next()).unwrap_or("-");
        out.push_str(&format!("{name} | {args}\n"));
    }
    out
}

#[test]
fn ci_stage_table_is_unchanged() {
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).unwrap();
    assert_eq!(stage_table(&src), include_str!("golden/ci-stages.txt"));
}
