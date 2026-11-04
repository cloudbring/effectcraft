//! #188: OLD (pre `craft-keymap`) vs NEW shortcut resolution.
//!
//! OLD is the real engine at 7870b5916ae0749d497d880989683bf2946d6c8b (merge-base of this
//! branch with `origin/main`), linked as `effectcraft-engine-old`; its UI dispatch glue is vendored
//! in `old_dispatch.rs`. NEW is this crate's engine and `menus` code. Three layers:
//!   A. the binding tables: (chord text, scope, binding key) per chord, both profiles;
//!   B. the egui dispatch lists (modifiers, key, command, params);
//!   C. resolution of a key press: candidates + `pick`, over the key x modifier x focus x
//!      context grid (default and legacy presets) and over random presets/chords (proptest).

mod old_dispatch;

use std::collections::BTreeSet;

use craft_keymap::{Key, Mods, Platform, Trigger};
use effectcraft_engine::shortcuts as new;
use effectcraft_engine_old::shortcuts as old;
use effectcraft_ui_egui::menus::UI_COMMANDS;
use proptest::prelude::*;
use serde_json::{Value, json};

const FOCUS: &[&str] = &["Project", "Timeline", "Composition", "RenderQueue", "Effects"];
const CONTEXTS: &[&str] = &["TextEditing", "Paint", "Mask", "Camera"];
const LEGACY_FIXTURE: &str = include_str!("../../../../engine/src/tests/fixtures/shortcuts_v0_legacy.json");

fn old_ui() -> Vec<old::UiCommand> {
    UI_COMMANDS.iter().map(|c| old::UiCommand { id: c.id.into(), label: c.label.into(), shortcut: c.shortcut.map(str::to_string), scope: None }).collect()
}
fn new_ui() -> Vec<new::UiCommand> {
    UI_COMMANDS.iter().map(|c| new::UiCommand { id: c.id.into(), label: c.label.into(), shortcut: c.shortcut.map(str::to_string), scope: None }).collect()
}

#[derive(Clone, Copy, Debug)]
enum Profile {
    Mac,
    Windows,
}
impl Profile {
    fn mac(self) -> bool {
        matches!(self, Profile::Mac)
    }
    fn platform(self) -> Platform {
        if self.mac() { Platform::Mac } else { Platform::Windows }
    }
}
const PROFILES: [Profile; 2] = [Profile::Mac, Profile::Windows];

/// Off macOS the UI drops `Ctrl+Cmd` chords (see `bindings` in both versions).
fn reachable(text: &str, p: Profile) -> bool {
    p.mac() || !(text.split('+').any(|x| x == "Ctrl") && text.split('+').any(|x| x == "Cmd"))
}

/// A chord text as a spelling-independent token: the two versions print the same chord
/// differently (`ArrowLeft`/`Left`, `Ctrl+Cmd`/`Cmd+Ctrl`, `Num0`/`0`, `Num*`/`KeypadMultiply`).
/// Both are reduced to (physical?, cmd, ctrl, alt, shift, egui key); `Num*` has no egui key.
fn canon_old(text: &str) -> String {
    let (phys, rest) = match text.strip_prefix("Physical+") {
        Some(r) => (true, r),
        None => (false, text),
    };
    if rest.ends_with("Num*") {
        let mods: Vec<&str> = rest.trim_end_matches("Num*").split('+').filter(|m| !m.is_empty()).collect();
        return format!("{phys}|{mods:?}|Num*");
    }
    match old_dispatch::parse_shortcut(rest, true) {
        Some((m, k)) => format!("{phys}|cmd{} ctrl{} alt{} shift{}|{k:?}", m.command, m.ctrl, m.alt, m.shift),
        None => format!("{phys}|unparsed:{rest}"),
    }
}
fn canon_new(text: &str) -> String {
    let (phys, rest) = match text.strip_prefix("Physical+") {
        Some(r) => (true, r),
        None => (false, text),
    };
    match Trigger::parse(rest) {
        Ok(t) => {
            let m = t.chord.mods;
            match t.chord.to_egui_shortcut() {
                Some(sc) => format!("{phys}|cmd{} ctrl{} alt{} shift{}|{:?}", m.cmd, m.ctrl, m.alt, m.shift, sc.logical_key),
                None if t.chord.key == Key::KeypadMultiply => {
                    let mut mods: Vec<&str> = vec![];
                    for (on, n) in [(m.ctrl, "Ctrl"), (m.cmd, "Cmd"), (m.alt, "Alt"), (m.shift, "Shift")] {
                        if on {
                            mods.push(n);
                        }
                    }
                    format!("{phys}|{mods:?}|Num*")
                }
                None => format!("{phys}|unmapped:{rest}"),
            }
        }
        Err(_) => format!("{phys}|unparsed:{rest}"),
    }
}

/// Layer A rows: (canonical chord, scope, binding key).
fn old_rows(t: &old::ShortcutTable, p: Profile) -> Vec<(String, String, String)> {
    t.bindings().into_iter().filter(|(k, _)| reachable(k, p)).map(|(k, b)| (canon_old(k), b.scope.clone(), b.key.clone())).collect()
}
fn new_rows(t: &new::ShortcutTable, p: Profile) -> Vec<(String, String, String)> {
    t.bindings().into_iter().filter(|(k, _)| reachable(k, p)).map(|(k, b)| (canon_new(&k), b.scope.as_str().to_string(), b.command.key())).collect()
}

fn old_table(maps: &old::Keymaps) -> old::ShortcutTable {
    old::ShortcutTable::build(maps, &old_ui())
}
fn new_table(maps: &new::Keymaps, p: Profile) -> new::ShortcutTable {
    new::ShortcutTable::build_for_platform(maps, &new_ui(), &[], p.platform())
}

type Pressed = (egui::Key, egui::Modifiers);

fn pressed_mods(p: Profile, ctrl: bool, shift: bool, alt: bool, meta: bool) -> egui::Modifiers {
    if p.mac() {
        egui::Modifiers { alt, ctrl, shift, mac_cmd: meta, command: meta }
    } else {
        egui::Modifiers { alt, ctrl, shift, mac_cmd: false, command: ctrl }
    }
}
fn mods_grid(p: Profile) -> Vec<egui::Modifiers> {
    let mut v = vec![];
    for bits in 0..16u8 {
        let (ctrl, shift, alt, meta) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
        if !p.mac() && meta {
            continue;
        }
        v.push(pressed_mods(p, ctrl, shift, alt, meta));
    }
    v
}

/// What a press runs: `Ok(Some((command, params)))`, `Ok(None)`, or the tied scopes.
type Resolved = Result<Option<(String, Value)>, Vec<String>>;

fn norm_params(v: &Value) -> Value {
    if v.is_null() { json!({}) } else { v.clone() }
}

struct OldSide {
    table: old::ShortcutTable,
    binds: Vec<old_dispatch::Binding>,
    mac: bool,
}
impl OldSide {
    fn new(maps: &old::Keymaps, p: Profile) -> Self {
        let table = old_table(maps);
        let binds = old_dispatch::bindings(&table, p.mac(), true);
        OldSide { table, binds, mac: p.mac() }
    }
    fn resolve(&self, (key, mods): Pressed, focused: &str, contexts: &[&str]) -> Resolved {
        let mac = self.mac;
        let scope_of = |id: &str, params: &Value| {
            let k = old::binding_key(id, params);
            self.table.find(&k).map_or(old::APP_SCOPE, |b| b.scope.as_str())
        };
        let physical = self.table.bindings().into_iter().filter_map(|(sc, b)| {
            let (m, k) = old_dispatch::parse_shortcut(sc.strip_prefix("Physical+")?, mac)?;
            (k == key && old_dispatch::mods_match(m, mods, mac)).then_some((b.scope.as_str(), true, (b.command.clone(), b.params.clone())))
        });
        let logical = self
            .binds
            .iter()
            .filter(|(m, k, ..)| *k == key && old_dispatch::mods_match(*m, mods, mac))
            .map(|(_, _, id, params)| (scope_of(id, params), false, (id.clone(), params.clone())));
        old::pick(physical.chain(logical), focused, contexts).map(|c| c.map(|(id, p)| (id, norm_params(&p))))
    }
}

struct NewSide {
    table: new::ShortcutTable,
    binds: Vec<(craft_keymap::Chord, String, Value)>,
    platform: Platform,
}
impl NewSide {
    fn new(maps: &new::Keymaps, p: Profile) -> Self {
        let table = new_table(maps, p);
        let platform = p.platform();
        // Same filter chain as `chord_bindings` in ui-egui/src/menus/shortcuts.rs.
        let binds = table
            .bindings()
            .into_iter()
            .filter(|(sc, _)| !sc.starts_with("Physical+"))
            .filter_map(|(sc, b)| Some((Trigger::parse(&sc).ok()?.chord, b.command.id.clone(), b.command.params.clone())))
            .filter(|(c, ..)| platform.is_mac() || !(c.mods.cmd && c.mods.ctrl))
            .collect();
        NewSide { table, binds, platform }
    }
    fn resolve(&self, (key, mods): Pressed, focused: &str, contexts: &[&str]) -> Resolved {
        let platform = self.platform;
        let scope_of = |id: &str, params: &Value| {
            let k = new::binding_key(id, params);
            self.table.find(&k).map_or(new::APP_SCOPE, |b| b.scope.as_str())
        };
        let physical = self.table.bindings().into_iter().filter_map(|(sc, b)| {
            let chord = Trigger::parse(sc.strip_prefix("Physical+")?).ok()?.chord;
            let same = Key::from_egui(key) == Some(chord.key) && Mods::from_egui(&mods, platform) == chord.mods;
            same.then_some((b.scope.as_str(), true, (b.command.id.clone(), b.command.params.clone())))
        });
        let logical = self
            .binds
            .iter()
            .filter(|(c, ..)| craft_keymap::egui::matches(c, key, &mods, platform))
            .map(|(_, id, params)| (scope_of(id, params), false, (id.clone(), params.clone())));
        new::pick(physical.chain(logical), focused, contexts).map(|c| c.map(|(id, p)| (id, norm_params(&p))))
    }
}

fn context_sets() -> Vec<Vec<&'static str>> {
    (0..16u8).map(|m| CONTEXTS.iter().enumerate().filter(|(i, _)| m & (1 << i) != 0).map(|(_, c)| *c).collect()).collect()
}

/// A multiset diff of two row lists, for readable failures.
fn diff_rows(a: &[(String, String, String)], b: &[(String, String, String)]) -> String {
    let sa: BTreeSet<_> = a.iter().collect();
    let sb: BTreeSet<_> = b.iter().collect();
    format!("only old: {:?}\nonly new: {:?}", sa.difference(&sb).collect::<Vec<_>>(), sb.difference(&sa).collect::<Vec<_>>())
}

/// Layers A, B and the full press grid for one old/new preset pair.
fn assert_equivalent(label: &str, om: &old::Keymaps, nm: &new::Keymaps, grid: bool) {
    for p in PROFILES {
        let (o, n) = (OldSide::new(om, p), NewSide::new(nm, p));
        // A: tables.
        let (mut ra, mut rb) = (old_rows(&o.table, p), new_rows(&n.table, p));
        assert_eq!(o.table.bindables.iter().map(|b| b.key.clone()).collect::<Vec<_>>().len(), n.table.catalog().len(), "{label} {p:?}: catalog size");
        ra.sort();
        rb.sort();
        assert!(ra == rb, "{label} {p:?}: binding tables differ\n{}", diff_rows(&ra, &rb));
        // B: dispatch lists, order-insensitive (same-rank ties are covered by C).
        // Off macOS OLD sets `ctrl` as well as `command` for `Cmd`; NEW sets `command` only.
        let norm = |v: Vec<(egui::Modifiers, egui::Key, String, Value)>| {
            let mut v: Vec<String> = v
                .into_iter()
                .map(|(m, k, id, par)| {
                    format!("cmd{} ctrl{} alt{} shift{}|{k:?}|{id}|{}", m.command, m.ctrl && !(m.command && !p.mac()), m.alt, m.shift, norm_params(&par))
                })
                .collect();
            v.sort();
            v
        };
        let ob = norm(old_dispatch::bindings(&o.table, p.mac(), false));
        let nb = norm(n.binds.iter().filter_map(|(c, id, par)| c.to_egui_shortcut().map(|s| (s.modifiers, s.logical_key, id.clone(), par.clone()))).collect());
        // The shifted-alias duplicates (`Shift+=` also as `Plus`) are left out of the old list: new handles them
        // inside `craft_keymap::egui::matches`, which layer C exercises.
        let (so, sn): (BTreeSet<_>, BTreeSet<_>) = (ob.iter().collect(), nb.iter().collect());
        assert!(
            so == sn,
            "{label} {p:?}: dispatch lists differ\nonly old: {:?}\nonly new: {:?}",
            so.difference(&sn).collect::<Vec<_>>(),
            sn.difference(&so).collect::<Vec<_>>()
        );
        if !grid {
            continue;
        }
        // C: resolution over every key, modifier combination, focused panel and context set.
        let mut got = grid_diffs(&o, &n, p);
        let registered = remove_registered(&mut got, p, label);
        assert!(got.is_empty(), "{label} {p:?}: {} unregistered differing presses\n{}", got.len(), render_diffs(&got));
        if label == "default" {
            assert_eq!(registered, 3, "{label} {p:?}: the default table has exactly the 3 registered differences");
        }
    }
}

/// Differences by press (key + modifiers): which (old -> new) resolutions occur, and at which
/// focus. Empty when old and new agree over the whole grid.
type Diffs = std::collections::BTreeMap<String, BTreeSet<String>>;

fn grid_diffs(o: &OldSide, n: &NewSide, p: Profile) -> Diffs {
    // No context, each context alone, and all four: `pick` ranks contexts pairwise, and the random
    // probes cover the other subsets.
    let ctxs: Vec<Vec<&str>> = context_sets().into_iter().filter(|c| c.len() <= 1 || c.len() == CONTEXTS.len()).collect();
    let mut out = Diffs::new();
    for &key in egui::Key::ALL {
        for mods in mods_grid(p) {
            for focused in FOCUS {
                for c in &ctxs {
                    let (a, b) = (o.resolve((key, mods), focused, c), n.resolve((key, mods), focused, c));
                    if a != b {
                        let press = format!("{key:?} ctrl={} shift={} alt={} meta/cmd={}", mods.ctrl, mods.shift, mods.alt, mods.command);
                        let short = |r: &Resolved| match r {
                            Ok(Some((id, par))) => format!("{id} {par}"),
                            Ok(None) => "(nothing)".to_string(),
                            Err(t) => format!("(tie {t:?})"),
                        };
                        out.entry(press).or_default().insert(format!("focus={focused}: {} -> {}", short(&a), short(&b)));
                    }
                }
            }
        }
    }
    out
}

/// Keys whose press the NEW matcher (`craft_keymap::egui::matches`) reads more loosely than OLD
/// did: the shifted symbols a layout reports (`Plus`, `Exclamationmark`, `Colon`, ...). Random
/// probes skip them; the grid reports them (see `remove_registered`).
const LAYOUT_TOLERANT: &[egui::Key] = &[
    egui::Key::Plus,
    egui::Key::Equals,
    egui::Key::Exclamationmark,
    egui::Key::Colon,
    egui::Key::Questionmark,
    egui::Key::Pipe,
    egui::Key::OpenCurlyBracket,
    egui::Key::CloseCurlyBracket,
];

/// The differences between the two versions that are known and understood. Each is a consequence
/// of `craft_keymap::egui::matches` being layout tolerant ("a binding on `=` matches `+` with or
/// without Shift"), which the OLD dispatcher's shifted-alias table was not:
///   1. `Alt+Shift+!` (Alt+Shift+1 on a US keyboard) runs `view.channel.redColorized`; OLD ran nothing
///      because egui reports `!`, not `1`. A fix.
///   2. A bare `+` runs `timeline.zoomIn` (the `=` binding); OLD ran nothing. A fix.
///   3. `Shift+=` (reported as Shift+`+`) with the Timeline or Composition focused runs
///      `timeline.zoomIn`; OLD ran `layer.setBlendMode {step: 1}` (After Effects' "next blend mode").
///      A behaviour change that loses a default shortcut in those two panels; see the ignored test
///      `shift_equals_next_blend_mode_in_timeline_regressed`.
///   4. Off macOS a plain `Ctrl+..` chord is dead in OLD and is the same chord as `Cmd+..` in NEW.
///
/// Returns how many distinct presses were registered and removed from `d`.
fn remove_registered(d: &mut Diffs, p: Profile, label: &str) -> usize {
    let mut n = 0;
    d.retain(|press, lines| {
        let registered = |line: &str| match press.as_str() {
            "Exclamationmark ctrl=false shift=true alt=true meta/cmd=false" => line.ends_with(": (nothing) -> view.channel.redColorized {}"),
            "Plus ctrl=false shift=false alt=false meta/cmd=false" => line.ends_with(": (nothing) -> timeline.zoomIn {}"),
            "Plus ctrl=false shift=true alt=false meta/cmd=false" => {
                (line.starts_with("focus=Timeline:") || line.starts_with("focus=Composition:"))
                    && line.ends_with(": layer.setBlendMode {\"step\":1} -> timeline.zoomIn {}")
            }
            // 4. A plain `Ctrl+..` chord off macOS: OLD never matched it (a Windows Ctrl press also sets
            //    `command`, which `mods_match` requires to be false for `Ctrl`), NEW reads Ctrl as Cmd. The
            //    legacy fixture's `Ctrl+Alt+Y` (layer.newNull) now wins over the default `Cmd+Alt+Y`.
            "Y ctrl=true shift=false alt=true meta/cmd=true" if !p.mac() && label.starts_with("legacy") => {
                line.ends_with(": layer.newAdjustment {} -> layer.newNull {}")
            }
            _ => false,
        };
        lines.retain(|l| !registered(l));
        let keep = !lines.is_empty();
        if !keep {
            n += 1;
        }
        keep
    });
    n
}

/// `Shift+=` is After Effects' "next blend mode" (`layer.setBlendMode`), and it was one in OLD
/// with the Timeline or Composition focused. NEW's tolerant matcher lets the `=` zoom binding of
/// those panels claim it first. This is the regression half of difference 3; it fails today.
#[test]
#[ignore = "known regression of the craft-keymap swap, see remove_registered docs (difference 3)"]
fn shift_equals_next_blend_mode_in_timeline_regressed() {
    for p in PROFILES {
        let (o, n) = (OldSide::new(&old::Keymaps::default(), p), NewSide::new(&new::Keymaps::default(), p));
        let pressed = (egui::Key::Plus, pressed_mods(p, false, true, false, false));
        for focus in ["Timeline", "Composition"] {
            assert_eq!(o.resolve(pressed, focus, &[]), n.resolve(pressed, focus, &[]), "{p:?} focus={focus}");
        }
    }
}

fn render_diffs(d: &Diffs) -> String {
    d.iter().map(|(k, v)| format!("{k}\n    {}", v.iter().cloned().collect::<Vec<_>>().join("\n    "))).collect::<Vec<_>>().join("\n")
}

#[test]
fn default_preset_resolves_identically_on_mac_and_windows() {
    assert_equivalent("default", &old::Keymaps::default(), &new::Keymaps::default(), true);
}

#[test]
fn every_chord_of_the_old_default_table_resolves_to_the_same_command() {
    // Layer C restricted to the chords the shipped table actually binds, with the result
    // asserted to be a command (not just "same nothing"): old and new both run it.
    for p in PROFILES {
        let (o, n) = (OldSide::new(&old::Keymaps::default(), p), NewSide::new(&new::Keymaps::default(), p));
        let mut runs = 0;
        for (m, k, id, params) in o.binds.iter().filter(|(_, k, ..)| !LAYOUT_TOLERANT.contains(k)) {
            let want = Ok(Some((id.clone(), norm_params(params))));
            for focused in FOCUS {
                let a = o.resolve((*k, *m), focused, &[]);
                let b = n.resolve((*k, *m), focused, &[]);
                assert_eq!(a, b, "{p:?} {m:?} {k:?} focus={focused}");
                if a == want {
                    runs += 1;
                }
            }
        }
        assert!(runs > 100, "{p:?}: only {runs} chords ran their own command");
    }
}

fn legacy_names() -> Vec<String> {
    serde_json::from_str::<Value>(LEGACY_FIXTURE).unwrap()["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

/// The legacy fixture holds an un-normalized chord (`shift+cmd+k`). The OLD loader keeps it raw
/// (so the old dispatcher parses it as a bare `K`); the NEW loader normalizes it on migration.
/// This is the one registered difference; every other fixture chord resolves identically.
#[test]
fn legacy_fixture_presets_resolve_identically_except_the_raw_chord() {
    let doc: Value = serde_json::from_str(LEGACY_FIXTURE).unwrap();
    for name in legacy_names() {
        let mut om = old::Keymaps::from_json(LEGACY_FIXTURE);
        let mut nm = new::Keymaps::from_json(LEGACY_FIXTURE);
        om.select(&name).unwrap();
        nm.select(&name).unwrap();
        let raw_chords: Vec<String> = doc["presets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"] == name.as_str())
            .flat_map(|p| p["overrides"].as_object().unwrap().values().flat_map(|v| v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string())))
            .filter(|c| old::normalize(c).as_deref() != Some(c.as_str()))
            .collect();
        if raw_chords.is_empty() {
            assert_equivalent(&format!("legacy:{name}"), &om, &nm, true);
        } else {
            // Normalize the fixture the way the new loader does, hand it to the old one, and the
            // two agree: the difference is exactly the normalization.
            let mut fixed = doc.clone();
            for p in fixed["presets"].as_array_mut().unwrap() {
                for v in p["overrides"].as_object_mut().unwrap().values_mut() {
                    *v = json!(v.as_array().unwrap().iter().filter_map(|s| old::normalize(s.as_str().unwrap())).collect::<Vec<_>>());
                }
            }
            let mut om2 = old::Keymaps::from_json(&fixed.to_string());
            om2.select(&name).unwrap();
            assert_equivalent(&format!("legacy-normalized:{name}"), &om2, &nm, true);
            // And the raw load differs, which pins the behaviour change the swap introduced.
            let raw_old = old_rows(&old_table(&om), Profile::Mac);
            let new_r = new_rows(&new_table(&nm, Profile::Mac), Profile::Mac);
            assert_ne!(
                {
                    let mut r = raw_old;
                    r.sort();
                    r
                },
                {
                    let mut r = new_r;
                    r.sort();
                    r
                },
                "raw chords {raw_chords:?} now resolve the same: update the registered difference"
            );
        }
    }
}

/// A key name neither version knows: OLD `normalize` keeps it verbatim as the key (the chord can
/// never fire), NEW rejects the text (`set` reports "is not a shortcut"). Pinned so a change shows.
#[test]
fn normalize_unknown_key_names_differ() {
    // A repeated modifier is rejected by NEW too (OLD ignores the repeat).
    assert_eq!((old::normalize("Cmd+Cmd+A").as_deref(), new::normalize("Cmd+Cmd+A")), (Some("Cmd+A"), None));
    // `Num1` (a typed numpad name) stays `Num1` in OLD, which the dispatcher can't parse so the chord is
    // dead; NEW reads it as the digit key `1`. The shipped tables use `Num0` and `Num*` only.
    assert_eq!((old::normalize("Num1").as_deref(), new::normalize("Num1").as_deref()), (Some("Num1"), Some("1")));
    // `Win` is accepted by OLD as Cmd.
    assert_eq!(old::normalize("Win+A").as_deref(), Some("Cmd+A"));
    assert_eq!(new::normalize("Win+A"), None);
    for junk in ["aA", "Foo", "Cmd+Bar"] {
        assert!(old::normalize(junk).is_some(), "{junk}");
        assert!(new::normalize(junk).is_none(), "{junk}");
    }
}

/// `Physical+Ctrl+Cmd+A` is a macOS-only chord. OLD dropped it off macOS for logical bindings but not
/// for physical ones, so on Windows it ran as plain Ctrl+A (stealing Select All); NEW drops both.
#[test]
fn physical_ctrl_cmd_chord_off_macos() {
    let text = json!({"active": "P", "presets": [{"name": "P", "overrides": {"app.about": ["Physical+Ctrl+Cmd+A"]}}]}).to_string();
    let (om, nm) = (old::Keymaps::from_json(&text), new::Keymaps::from_json(&text));
    let p = Profile::Windows;
    let pressed = (egui::Key::A, pressed_mods(p, true, false, false, false));
    let (o, n) = (OldSide::new(&om, p).resolve(pressed, "Timeline", &[]), NewSide::new(&nm, p).resolve(pressed, "Timeline", &[]));
    assert_eq!(o, Ok(Some(("app.about".to_string(), json!({})))));
    assert_eq!(n, Ok(Some(("edit.selectAll".to_string(), json!({})))));
}

// ---- random chords and presets ------------------------------------------------------------

const KEYS: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "Space",
    "Escape",
    "Enter",
    "Tab",
    "Backspace",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    ";",
    "'",
    ",",
    ".",
    "/",
    "\\",
    "-",
    "`",
    "[",
    "]",
    "Num0",
    "Num*",
];

fn chord_text(bits: u8, key: usize, physical: bool) -> String {
    let mut s = String::new();
    if physical {
        s.push_str("Physical+");
    }
    for (i, m) in ["Ctrl+", "Cmd+", "Alt+", "Shift+"].iter().enumerate() {
        if bits & (1 << i) != 0 {
            s.push_str(m);
        }
    }
    s.push_str(KEYS[key % KEYS.len()]);
    s
}

fn chord() -> impl Strategy<Value = String> {
    // No `Physical+Ctrl+Cmd+..`: a macOS-only chord that OLD still bound off macOS when physical
    // (see `physical_ctrl_cmd_chord_off_macos`).
    (0..16u8, 0..KEYS.len(), prop::bool::weighted(0.1)).prop_map(|(b, k, ph)| chord_text(if ph { b & !1 } else { b }, k, ph))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// `normalize` agrees (as a chord, not as spelling) on shortcut text written with the aliases,
    /// casing, modifier order and glyphs people type. Unknown key names are excluded: see
    /// `normalize_unknown_key_names_differ`.
    #[test]
    fn normalize_agrees_on_typed_variants(
        mods in prop::collection::vec(0..14usize, 0..4), key in 0..KEYS.len(), lower in any::<bool>(), glyph in any::<bool>(),
    ) {
        const MODS: [&str; 14] = ["Ctrl", "ctrl", "Control", "Cmd", "cmd", "Command", "Meta", "Super", "Super", "Alt", "opt", "Option", "Shift", "shift"];
        let mut mods = mods;
        const GROUP: [u8; 14] = [0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3];
        mods.sort_by_key(|m| GROUP[*m]);
        mods.dedup_by_key(|m| GROUP[*m]);
        let k = KEYS[key];
        let k = if lower { k.to_lowercase() } else { k.to_string() };
        let text = if glyph && mods.len() <= 1 && k.chars().count() == 1 && k != "+" {
            format!("{}{k}", ["⌃", "⌥", "⇧", "⌘", ""][mods.first().map_or(4, |m| m % 5)])
        } else {
            let mut parts: Vec<String> = mods.iter().map(|&m| MODS[m].to_string()).collect();
            parts.push(k);
            parts.join("+")
        };
        let (o, n) = (old::normalize(&text), new::normalize(&text));
        prop_assert_eq!(o.as_deref().map(canon_old), n.as_deref().map(canon_new), "{:?}: old {:?} new {:?}", text, o, n);
    }

    #[test]
    fn normalize_agrees_on_canonical_chords(c in chord()) {
        let (o, n) = (old::normalize(&c), new::normalize(&c));
        prop_assert_eq!(o.as_deref().map(canon_old), n.as_deref().map(canon_new), "{:?}", c);
        // Idempotent in both.
        let (o2, n2) = (old::normalize(&o.clone().unwrap()), new::normalize(&n.clone().unwrap()));
        prop_assert_eq!(o2, o);
        prop_assert_eq!(n2, n);
    }

    /// Random chords against the default table: same resolution, any focus and contexts.
    #[test]
    fn random_chords_resolve_identically_on_default(
        bits in 0..16u8, key in 0..egui::Key::ALL.len(), prof in 0..2usize, focus in 0..FOCUS.len(), ctx in 0..16usize,
    ) {
        let p = PROFILES[prof];
        let mods = mods_grid(p)[bits as usize % mods_grid(p).len()];
        let ctxs = &context_sets()[ctx];
        let (o, n) = (OldSide::new(&old::Keymaps::default(), p), NewSide::new(&new::Keymaps::default(), p));
        if !LAYOUT_TOLERANT.contains(&egui::Key::ALL[key]) {
            let pressed = (egui::Key::ALL[key], mods);
            prop_assert_eq!(o.resolve(pressed, FOCUS[focus], ctxs), n.resolve(pressed, FOCUS[focus], ctxs));
        }
    }

    /// Random custom presets (normalized chords, as `shortcuts.set` writes them), loaded from the
    /// legacy JSON by both loaders: identical tables and identical resolution of every chord.
    #[test]
    fn random_presets_resolve_identically(
        edits in prop::collection::vec((any::<prop::sample::Index>(), prop::collection::vec(chord(), 0..3)), 0..12),
        probes in prop::collection::vec((0..16u8, 0..egui::Key::ALL.len(), 0..FOCUS.len(), 0..16usize), 24),
    ) {
        let base = OldSide::new(&old::Keymaps::default(), Profile::Mac);
        let keys: Vec<String> = base.table.bindables.iter().map(|b| b.key.clone()).collect();
        let mut overrides = serde_json::Map::new();
        for (ix, chords) in &edits {
            let key = keys[ix.index(keys.len())].clone();
            let norm: Vec<String> = chords.iter().filter_map(|c| old::normalize(c.strip_prefix("Physical+").unwrap_or(c)).map(|n| if c.starts_with("Physical+") { format!("Physical+{n}") } else { n })).collect();
            overrides.insert(key, json!(norm));
        }
        for p in PROFILES {
            // Off macOS a plain `Ctrl+..` chord is dead in OLD and equals `Cmd+..` in NEW (difference 4
            // in `remove_registered`), so the Windows leg uses the preset without them.
            let mut ov = overrides.clone();
            if !p.mac() {
                for v in ov.values_mut() {
                    let keep: Vec<Value> = v.as_array().unwrap().iter().filter(|c| { let t = c.as_str().unwrap(); !(t.contains("Ctrl+") && !t.contains("Cmd+")) }).cloned().collect();
                    *v = Value::Array(keep);
                }
            }
            let text = json!({"active": "P", "presets": [{"name": "P", "overrides": ov}]}).to_string();
            let (om, nm) = (old::Keymaps::from_json(&text), new::Keymaps::from_json(&text));
            let (o, n) = (OldSide::new(&om, p), NewSide::new(&nm, p));
            let (mut ra, mut rb) = (old_rows(&o.table, p), new_rows(&n.table, p));
            ra.sort();
            rb.sort();
            prop_assert!(ra == rb, "{p:?}: tables differ\n{}", diff_rows(&ra, &rb));
            for &(b, k, f, c) in &probes {
                let grid = mods_grid(p);
                if LAYOUT_TOLERANT.contains(&egui::Key::ALL[k]) {
                    continue;
                }
                let pressed = (egui::Key::ALL[k], grid[b as usize % grid.len()]);
                let ctxs = &context_sets()[c];
                prop_assert_eq!(o.resolve(pressed, FOCUS[f], ctxs), n.resolve(pressed, FOCUS[f], ctxs), "{:?} {:?}", p, pressed);
            }
            // Every chord the preset binds, at every focus: probed directly.
            for (m, k, ..) in o.binds.iter().filter(|(_, k, ..)| !LAYOUT_TOLERANT.contains(k)).take(400) {
                for f in FOCUS {
                    prop_assert_eq!(o.resolve((*k, *m), f, &[]), n.resolve((*k, *m), f, &[]), "{:?} {:?} {:?}", p, m, k);
                }
            }
        }
    }
}
