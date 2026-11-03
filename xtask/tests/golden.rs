use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_xtask::{Cargo, ico, layers, pdf_deps, version};
use serde_json::{Value, json};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("effectcraft-70-golden-{}-{}", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, rel: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn xtask_cmd(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask")).args(args).current_dir(cwd).env("CARGO_MANIFEST_DIR", Path::new(env!("CARGO_MANIFEST_DIR"))).output().unwrap()
}

fn assert_output(output: Output, expected: &[u8]) {
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, expected);
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}

fn ec(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| if *v == "effectcraft" { (*v).into() } else { format!("effectcraft-{v}") }).collect()
}

fn version_config(root: &Path) -> version::Config {
    version::Config { cargo: Cargo { root: root.into(), ..Cargo::default() }, ..version::Config::default() }
}

fn layers_config(root: &Path) -> layers::Config {
    layers::Config {
        root: root.into(),
        layers: [
            ("L0", &["time", "geom", "color", "vp9enc", "testkit", "opusenc", "hevcenc", "av1enc"][..]),
            ("L1", &["raster", "keyframe", "path", "psd", "segment"]),
            ("L2", &["project", "text", "effects", "track", "svg", "pdf", "model"]),
            ("L3", &["render", "media", "expr", "export", "gpu", "sam2", "lottie", "format", "plugin", "interchange"]),
            ("L4", &["engine", "script", "host"]),
            ("L5", &["ui-egui", "automation"]),
            ("L6", &["effectcraft", "cli", "web"]),
        ]
        .into_iter()
        .map(|(label, crates)| (label.into(), ec(crates)))
        .collect(),
        exempt: names(&["xtask"]),
        ignore_dev: true,
        allowed_edges: [
            ("path", "keyframe"),
            ("path", "raster"),
            ("text", "path"),
            ("pdf", "svg"),
            ("pdf", "text"),
            ("effects", "project"),
            ("effects", "text"),
            ("effects", "path"),
            ("effects", "track"),
            ("media", "render"),
            ("expr", "render"),
            ("export", "render"),
            ("export", "media"),
            ("gpu", "render"),
            ("lottie", "format"),
            ("host", "engine"),
            ("script", "engine"),
            ("host", "script"),
            ("cli", "effectcraft"),
        ]
        .into_iter()
        .map(|(a, b)| (format!("effectcraft-{a}"), if b == "effectcraft" { b.into() } else { format!("effectcraft-{b}") }))
        .collect(),
        external_rules: vec![layers::ExternalRule {
            dependencies: names(&["craft-ui", "egui", "eframe", "egui-wgpu", "winit", "rfd", "cpal", "muda"]),
            allowed_from: ec(&["ui-egui", "automation", "effectcraft", "cli", "web"]),
            allowed_dev_from: Vec::new(),
        }],
        ..layers::Config::default()
    }
}

const PDF_REV: &str = "92d392cff132eaf53e13afff77f38b5c11b73aa0";

fn pdf_deps_config(root: &Path) -> pdf_deps::Config {
    let source = pdf_deps::git_source("https://github.com/storytold/craft-libs", PDF_REV);
    pdf_deps::Config {
        cargo: Cargo { root: root.into(), ..Cargo::default() },
        packages: [("craft-pdf", "0.1.0", Some(&["read"][..])), ("hayro-syntax", "0.7.2", Some(&["std"][..])), ("hayro-jbig2", "0.3.0", None)]
            .into_iter()
            .map(|(name, version, exact)| pdf_deps::PackagePin {
                name: name.into(),
                version: version.into(),
                source: source.clone(),
                required_features: Vec::new(),
                exact_features: exact.map(names),
            })
            .collect(),
        forbidden_active: names(&["hayro", "hayro-interpret", "hayro-cmap", "krilla"]),
        ..pdf_deps::Config::default()
    }
}

#[test]
fn layers_matches_golden_and_enforces_effectcraft_rules() {
    assert_output(xtask_cmd(&["layers"], &repo_root()), include_bytes!("golden/layers.stdout"));

    let cfg = layers_config(&repo_root());
    let edge = |from: &str, to: &str, kind: layers::DepKind, workspace: bool| layers::Crate {
        name: from.into(),
        deps: vec![layers::Dep { name: to.into(), kind, workspace }],
    };
    assert!(layers::check(&cfg, &[edge("effectcraft-path", "effectcraft-keyframe", layers::DepKind::Normal, true)]).unwrap().is_empty());
    assert!(layers::check(&cfg, &[edge("effectcraft-cli", "effectcraft", layers::DepKind::Normal, true)]).unwrap().is_empty());
    assert!(layers::check(&cfg, &[edge("effectcraft-time", "effectcraft-engine", layers::DepKind::Dev, true)]).unwrap().is_empty());
    assert!(layers::check(&cfg, &[edge("effectcraft-ui-egui", "egui", layers::DepKind::Normal, false)]).unwrap().is_empty());
    for bad in [
        edge("effectcraft-keyframe", "effectcraft-path", layers::DepKind::Normal, true),
        edge("effectcraft-project", "effectcraft-engine", layers::DepKind::Normal, true),
        edge("effectcraft-engine", "egui", layers::DepKind::Normal, false),
        edge("effectcraft-render", "cpal", layers::DepKind::Normal, false),
        layers::Crate { name: "effectcraft-mystery".into(), deps: Vec::new() },
    ] {
        assert!(!layers::check(&cfg, &[bad]).unwrap().is_empty());
    }
}

#[test]
fn pdf_deps_matches_golden_and_enforces_parser_only_pins() {
    assert_output(xtask_cmd(&["pdf-deps"], &repo_root()), include_bytes!("golden/pdf-deps.stdout"));

    let cfg = pdf_deps_config(&repo_root());
    let source = pdf_deps::git_source("https://github.com/storytold/craft-libs", PDF_REV);
    let base = || {
        json!({
            "packages": [
                {"id": "craft-pdf", "name": "craft-pdf", "version": "0.1.0", "source": source},
                {"id": "hayro-syntax", "name": "hayro-syntax", "version": "0.7.2", "source": source},
                {"id": "hayro-jbig2", "name": "hayro-jbig2", "version": "0.3.0", "source": source}
            ],
            "resolve": {
                "nodes": [
                    {"id": "hayro-syntax", "features": ["std"]},
                    {"id": "craft-pdf", "features": ["read"]}
                ]
            }
        })
    };
    assert!(pdf_deps::validate(&cfg, &base()).is_ok());

    let mut m = base();
    m["packages"][0]["source"] = json!("registry");
    assert!(pdf_deps::validate(&cfg, &m).is_err());

    let mut m = base();
    let dup = m["packages"][0].clone();
    m["packages"].as_array_mut().unwrap().push(dup);
    assert!(pdf_deps::validate(&cfg, &m).is_err());

    let mut m = base();
    m["packages"].as_array_mut().unwrap().retain(|p| p["name"] != "hayro-syntax");
    assert!(pdf_deps::validate(&cfg, &m).is_err());

    let mut m = base();
    m["packages"][0]["source"] = Value::Null;
    assert!(pdf_deps::validate(&cfg, &m).is_err());

    for (index, feature) in [(0, "images"), (1, "write"), (1, "raster")] {
        let mut m = base();
        m["resolve"]["nodes"][index]["features"].as_array_mut().unwrap().push(json!(feature));
        assert!(pdf_deps::validate(&cfg, &m).is_err());
    }

    let mut m = base();
    m["resolve"]["nodes"].as_array_mut().unwrap().pop();
    assert!(pdf_deps::validate(&cfg, &m).is_err());

    for name in ["hayro", "hayro-interpret", "hayro-cmap", "krilla"] {
        let mut m = base();
        m["packages"].as_array_mut().unwrap().push(json!({"id": name, "name": name}));
        m["resolve"]["nodes"].as_array_mut().unwrap().push(json!({"id": name}));
        assert!(pdf_deps::validate(&cfg, &m).is_err());
    }

    let mut m = base();
    m["resolve"] = Value::Null;
    assert!(pdf_deps::validate(&cfg, &m).is_err());
}

#[test]
fn version_show_and_dry_run_plan_match_golden() {
    assert_output(xtask_cmd(&["version"], &repo_root()), include_bytes!("golden/version.stdout"));

    let temp = Temp::new();
    let root_manifest = std::fs::read(repo_root().join("Cargo.toml")).unwrap();
    temp.write("Cargo.toml", &root_manifest);

    let cfg = version_config(&temp.0);
    let show_plan = version::compute(&cfg, &version::Args::Show).unwrap();
    assert_eq!(version::render(&show_plan).as_bytes(), include_bytes!("golden/version.stdout"));

    let set_plan = version::compute(&cfg, &version::Args::Set("1.2.3-rc.1".into())).unwrap();
    assert_eq!(version::render(&set_plan).as_bytes(), include_bytes!("golden/version-set.stdout"));
    assert_eq!(set_plan.edits.len(), 1);
    assert_eq!(set_plan.edits[0].2.as_bytes(), include_bytes!("golden/version-after.toml"));
    assert_eq!(std::fs::read(temp.0.join("Cargo.toml")).unwrap(), root_manifest);

    let sample = "[workspace.package]\nversion = \"0.1.0\"\n";
    for ok in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2"] {
        let updated = version::rewrite(&cfg, sample, ok).unwrap();
        assert_eq!(version::read(&updated).unwrap(), ok);
    }
    for bad in ["1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0+meta", "1.a.0", ""] {
        assert!(version::rewrite(&cfg, sample, bad).is_err(), "{bad}");
    }
    let crlf = sample.replace('\n', "\r\n");
    assert_eq!(version::rewrite(&cfg, &crlf, "2.0.0").unwrap(), crlf.replace("0.1.0", "2.0.0"));
}

#[test]
fn ico_stdout_and_bytes_match_golden_and_committed_icon_is_valid() {
    let temp = Temp::new();
    for size in [16_u32, 32, 48, 256] {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&size.to_be_bytes());
        png.extend_from_slice(&size.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 1, 2, 3, 4]);
        temp.write(format!("{size}.png"), png);
    }
    assert_output(xtask_cmd(&["ico", "out.ico", "16.png", "32.png", "48.png", "256.png"], &temp.0), include_bytes!("golden/ico.stdout"));
    let actual: String = std::fs::read(temp.0.join("out.ico")).unwrap().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(actual, include_str!("golden/ico.hex").trim_end());
    assert!(ico::pack(&[]).is_err());

    let path = repo_root().join("assets/app-icon/effectcraft.ico");
    if let Ok(ico_bytes) = std::fs::read(&path) {
        assert_eq!(&ico_bytes[..4], &[0, 0, 1, 0]);
        let n = u16::from_le_bytes([ico_bytes[4], ico_bytes[5]]) as usize;
        assert!(n >= 4);
        for i in 0..n {
            let e = &ico_bytes[6 + i * 16..6 + (i + 1) * 16];
            let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            assert!(ico::png_size(&ico_bytes[off..off + len]).is_ok());
        }
    }
}

#[test]
fn help_includes_shared_task_syntax() {
    let out = xtask_cmd(&["--help"], &repo_root());
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for expected in [layers::cli_help(), pdf_deps::cli_help(), ico::cli_help(), version::cli_help()] {
        assert!(text.contains(expected), "missing {expected}");
    }
}
