//! Keyboard shortcut presets (`shortcuts.json`, `shortcuts.*` commands) backed by `craft-keymap`.
//!
//! Presets live in [`Keymaps`] ([`Profiles`]): the built-in read-only preset ([`DEFAULT_PRESET`])
//! and custom presets stored as differences from the default. Editing a shortcut while the default
//! preset is active creates a custom preset automatically. The shortcut dispatcher and the menus
//! read the active preset through [`ShortcutTable`] ([`Keymap`]).

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::LazyLock;

pub use craft_keymap::{
    Active, Bindable, Binding, Builtins, Chord, CommandRef, ConflictKind, Key, Keymap, Lookup, Overflow, Overrides, Platform, Policy, ProfileLimits, Profiles,
    Scope, ScopeKind, Scopes, Trigger,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Preset file in the config store.
pub const SHORTCUTS_FILE: &str = "shortcuts.json";
/// The built-in (read-only) preset.
pub const DEFAULT_PRESET: &str = "EffectCraft Default";
/// Scope of application-wide shortcuts.
pub const APP_SCOPE: &str = "Application";

/// Scopes that are a state of the app rather than a panel: editing text in the viewer, a paint
/// tool active, a mask selected, a camera tool active.
pub const CONTEXT_SCOPES: &[&str] = &["TextEditing", "Paint", "Mask", "Camera"];

/// Engine commands and menu entries whose shortcuts belong to one panel or context (binding key
/// → scope). Every other one is application-wide; frontend commands state their own.
const SCOPES: &[(&str, &str)] = &[
    ("file.rememberInterpretation", "Project"),
    ("file.applyInterpretation", "Project"),
    ("project.deleteWithoutConfirmation", "Project"),
    ("renderQueue.duplicate", "RenderQueue"),
];

static BUILTINS: LazyLock<[(&'static str, Overrides); 1]> = LazyLock::new(|| [(DEFAULT_PRESET, Overrides::new())]);

/// Built-in shortcut sets passed to [`Profiles`] and the shared shortcuts dialog.
pub fn builtins() -> &'static [(&'static str, Overrides)] {
    &*BUILTINS
}

/// A frontend-only command offered for binding (tools, timeline navigation…).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiCommand {
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    /// Its scope; none: from its id (`timeline.*` → Timeline, `view.*` → Composition).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Scope of an engine command or menu entry from its binding key.
pub fn scope_of(key: &str) -> &'static str {
    SCOPES.iter().find(|(k, _)| *k == key).map_or(APP_SCOPE, |(_, s)| s)
}

pub fn is_context(scope: &str) -> bool {
    CONTEXT_SCOPES.contains(&scope)
}

/// The Timeline and the Composition panel show the same composition and share their shortcuts:
/// the layer keys work with either one focused.
pub fn panels_share(a: &str, b: &str) -> bool {
    matches!((a, b), ("Timeline", "Composition") | ("Composition", "Timeline"))
}

/// A scope as shown to people (`RenderQueue` → `Render Queue`).
pub fn scope_label(scope: &str) -> String {
    let mut out = String::with_capacity(scope.len() + 2);
    for (i, c) in scope.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// The scope registry for [`Keymap`]: contexts, panels, and Timeline–Composition sharing.
pub fn scopes() -> Scopes {
    let mut s =
        Scopes::new().with_panel("Project").with_panel("Timeline").with_panel("Composition").with_panel("RenderQueue").with_share("Timeline", "Composition");
    for &c in CONTEXT_SCOPES {
        s = s.with_context(c);
    }
    s
}

/// How strongly a binding in `scope` claims a chord while the panel `focused` has the keyboard
/// and `contexts` are active: lower wins, `None` doesn't run at all.
pub fn scope_rank(scope: &str, focused: &str, contexts: &[&str]) -> Option<u8> {
    let s = Scope::new(scope)?;
    scopes().rank(&s, &Active { panel: Some(focused), contexts, typing: false })
}

/// Which of the bindings matching one key press runs: `candidates` are (scope, physical,
/// binding), in the dispatcher's order.
pub fn pick<'a, T>(candidates: impl IntoIterator<Item = (&'a str, bool, T)>, focused: &str, contexts: &[&str]) -> Result<Option<T>, Vec<String>> {
    let mut best: Option<((u8, bool), &str, T)> = None;
    let mut tied: Vec<String> = vec![];
    for (scope, physical, b) in candidates {
        let Some(rank) = scope_rank(scope, focused, contexts) else { continue };
        let at = (rank, !physical);
        match &best {
            Some((r, s, _)) if *r < at || (*r == at && *s == scope) => {}
            Some((r, _, _)) if *r == at => {
                if !tied.iter().any(|t| t == scope) {
                    tied.push(scope.to_string());
                }
            }
            _ => {
                tied = vec![scope.to_string()];
                best = Some((at, scope, b));
            }
        }
    }
    if tied.len() > 1 { Err(tied) } else { Ok(best.map(|(.., b)| b)) }
}

/// The binding key of a command with bound parameters.
pub fn binding_key(command: &str, params: &Value) -> String {
    CommandRef::with_params(command, params.clone()).key()
}

/// Scope of a frontend command from its id.
fn ui_scope(id: &str) -> &'static str {
    if id.starts_with("timeline.") {
        "Timeline"
    } else if id.starts_with("view.") {
        "Composition"
    } else {
        APP_SCOPE
    }
}

fn strip_physical(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix("Physical+").or_else(|| s.strip_prefix("physical+")).unwrap_or(s)
}

/// Canonical shortcut text (`cmd+shift+k`, `Shift+Cmd+K`, `⌘⇧K` → `Cmd+Shift+K`); `None` if it
/// has no key.
pub fn normalize(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("Physical+").or_else(|| s.strip_prefix("physical+")) {
        return Trigger::parse(rest).ok().map(|t| format!("Physical+{t}"));
    }
    Trigger::parse(s).ok().map(|t| t.to_string())
}

/// Parse a shortcut string into a [`Binding`].
pub fn parse_binding(s: &str) -> Option<Binding> {
    Binding::parse(strip_physical(s)).ok()
}

fn keep_for_platform(b: &Binding, platform: Platform) -> bool {
    platform.is_mac() || !(b.trigger.chord.mods.cmd && b.trigger.chord.mods.ctrl)
}

/// Every bindable command: menu entries, registry commands outside the menus, then the
/// frontend's own commands.
pub fn bindables(ui: &[UiCommand]) -> Vec<Bindable> {
    bindables_for_platform(ui, Platform::current())
}

/// [`bindables`] filtered for `platform` (macOS-only `Ctrl+Cmd` defaults are omitted elsewhere).
pub fn bindables_for_platform(ui: &[UiCommand], platform: Platform) -> Vec<Bindable> {
    let mut out: Vec<Bindable> = vec![];
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut push = |mut b: Bindable, out: &mut Vec<Bindable>| {
        b.defaults.retain(|d| keep_for_platform(d, platform));
        let key = b.command.key();
        if let Some(&i) = seen.get(&key) {
            for d in b.defaults {
                if !out[i].defaults.contains(&d) {
                    out[i].defaults.push(d);
                }
            }
            for p in b.paths {
                if !out[i].paths.contains(&p) {
                    out[i].paths.push(p);
                }
            }
            return;
        }
        seen.insert(key, out.len());
        out.push(b);
    };
    let entries = crate::menus::entries();
    for (path, e) in &entries {
        let cmd = CommandRef::with_params(&e.command, e.params.clone());
        let key = cmd.key();
        let alt = if e.params.is_null() { crate::commands::find(&e.command).map_or(&[][..], |c| c.alt_shortcuts) } else { &[] };
        let scope = Scope::new(scope_of(&key)).unwrap_or_else(Scope::app);
        let mut b = Bindable::new(cmd, &e.label, scope).with_path(path.clone());
        for d in e.shortcut.as_deref().into_iter().chain(e.alt_shortcuts.iter().map(String::as_str)).chain(alt.iter().copied()).filter_map(parse_binding) {
            b = b.with_default(d);
        }
        push(b, &mut out);
    }
    let recent = json!({"recent": 0});
    let mut recent_b = Bindable::new(CommandRef::with_params("file.runScript", recent), "Run Most Recent Script", Scope::app()).with_path(vec![
        "File".into(),
        "Scripts".into(),
        "Recent Script Files".into(),
    ]);
    if let Some(d) = parse_binding("Cmd+Alt+Shift+D") {
        recent_b = recent_b.with_default(d);
    }
    push(recent_b, &mut out);
    for c in crate::command_specs() {
        if !c.journal || entries.iter().any(|(_, e)| e.command == c.id) {
            continue;
        }
        let scope = Scope::new(scope_of(c.id)).unwrap_or_else(Scope::app);
        let mut b = Bindable::new(CommandRef::new(c.id), c.label, scope).with_path(vec!["Commands".into()]);
        for d in c.shortcut.into_iter().chain(c.alt_shortcuts.iter().copied()).filter_map(parse_binding) {
            b = b.with_default(d);
        }
        push(b, &mut out);
    }
    for c in ui {
        let scope_name = c.scope.as_deref().unwrap_or_else(|| ui_scope(&c.id));
        let cat = if scope_name == APP_SCOPE { "Tools & Panels".to_string() } else { scope_label(scope_name) };
        let scope = Scope::new(scope_name).unwrap_or_else(Scope::app);
        let mut b = Bindable::new(CommandRef::new(&c.id), &c.label, scope).with_path(vec![cat]);
        if let Some(d) = c.shortcut.as_deref().and_then(parse_binding) {
            b = b.with_default(d);
        }
        push(b, &mut out);
    }
    out
}

/// The File ▸ Scripts entries of the scripts `names` (file names, as the menu lists them), with
/// no default keys.
pub fn script_bindables(names: &[String]) -> Vec<Bindable> {
    names
        .iter()
        .map(|n| {
            let params = json!({"name": n});
            Bindable::new(CommandRef::with_params("file.runScript", params), n, Scope::app()).with_path(vec!["File".into(), "Scripts".into()])
        })
        .collect()
}

/// The saved shortcut sets and which one is active, backed by [`Profiles`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keymaps {
    pub profiles: Profiles,
}

impl Default for Keymaps {
    fn default() -> Self {
        let mut profiles = Profiles::with_limits(ProfileLimits::default());
        let _ = profiles.select(DEFAULT_PRESET, builtins());
        Keymaps { profiles }
    }
}

impl Deref for Keymaps {
    type Target = Profiles;
    fn deref(&self) -> &Profiles {
        &self.profiles
    }
}

impl DerefMut for Keymaps {
    fn deref_mut(&mut self) -> &mut Profiles {
        &mut self.profiles
    }
}

/// A conflict: another command already using the keys in an overlapping scope.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Conflict {
    pub keys: String,
    pub key: String,
    pub label: String,
    pub scope: String,
    pub overrides: bool,
}

fn normalize_override_map(v: &Value) -> Value {
    let Some(obj) = v.as_object() else { return json!({}) };
    let mut out = serde_json::Map::new();
    for (k, val) in obj {
        if let Some(arr) = val.as_array() {
            let norm: Vec<Value> = arr.iter().filter_map(|s| s.as_str().and_then(normalize).map(Value::String)).collect();
            out.insert(k.clone(), Value::Array(norm));
        }
    }
    Value::Object(out)
}

impl Keymaps {
    /// Load `shortcuts.json`, migrating the legacy `{active, presets:[{name, overrides}]}` format
    /// one-way into [`Profiles`] `{version, active, sets}` when encountered.
    pub fn from_json(text: &str) -> Keymaps {
        let converted = serde_json::from_str::<Value>(text).ok().and_then(|v| {
            let presets = v.get("presets")?.as_array()?;
            let active = v.get("active").and_then(Value::as_str).unwrap_or(DEFAULT_PRESET);
            let mut sets = Vec::with_capacity(presets.len());
            for p in presets {
                let name = p.get("name").and_then(Value::as_str).unwrap_or_default();
                let overrides = p.get("overrides").map(normalize_override_map).unwrap_or_else(|| json!({}));
                if name == DEFAULT_PRESET && overrides.as_object().is_some_and(|m| m.is_empty()) {
                    continue;
                }
                sets.push(json!({"name": name, "overrides": overrides}));
            }
            Some(json!({"version": 1, "active": active, "sets": sets}).to_string())
        });
        let raw = converted.as_deref().unwrap_or(text);
        let Ok(mut profiles) = Profiles::load(raw, ProfileLimits::default(), Overflow::TruncateNames) else {
            return Keymaps::default();
        };
        profiles.reconcile(builtins());
        if profiles.get(profiles.active(), builtins()).is_none() {
            let _ = profiles.select(DEFAULT_PRESET, builtins());
        }
        Keymaps { profiles }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.profiles).unwrap_or_default()
    }

    pub fn active_name(&self) -> &str {
        self.profiles.active_name(builtins())
    }

    /// Preset names, the default first.
    pub fn names(&self) -> Vec<String> {
        self.profiles.names(builtins())
    }

    pub fn active_overrides(&self) -> &Overrides {
        self.profiles.active_overrides(builtins())
    }

    /// Set the keys of a bindable (normalized; empty = no shortcut). Returns a newly created
    /// preset name when the default was active.
    pub fn set(&mut self, b: &Bindable, keys: &[String]) -> Result<Option<String>, String> {
        let mut norm = Vec::new();
        for k in keys {
            let n = normalize(k).ok_or_else(|| format!("`{k}` is not a shortcut"))?;
            if !norm.contains(&n) {
                norm.push(n);
            }
        }
        let mut ov = self.active_overrides().clone();
        let def_norm: Vec<String> = b.defaults.iter().map(ToString::to_string).collect();
        if norm == def_norm {
            ov.remove(&b.command.key());
        } else {
            ov.insert(b.command.key(), norm);
        }
        self.profiles.store(ov, builtins(), "Custom").map_err(|e| e.to_string())
    }

    /// Clear the active preset's changes (its shortcuts become the defaults).
    pub fn reset_active(&mut self) {
        if !Profiles::is_builtin(self.active_name(), builtins()) {
            let _ = self.profiles.store(Overrides::new(), builtins(), "Custom");
        }
    }

    pub fn select(&mut self, name: &str) -> Result<(), String> {
        self.profiles.select(name, builtins()).map_err(|_| format!("no shortcut preset `{name}`"))
    }

    /// Duplicate a preset (the default or a custom one) under a new name and activate it.
    pub fn duplicate(&mut self, from: &str, name: Option<&str>) -> Result<String, String> {
        let copy = format!("{from} copy");
        let to = name.unwrap_or(&copy);
        self.profiles.duplicate(from, to, builtins()).map_err(|e| e.to_string())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        self.profiles.delete(name, builtins()).map_err(|e| e.to_string())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.profiles.rename(from, to, builtins()).map(|_| ()).map_err(|e| e.to_string())
    }

    /// A preset as a portable JSON document.
    pub fn export(&self, name: Option<&str>) -> Result<Value, String> {
        let name = name.unwrap_or_else(|| self.active_name());
        let text = self.profiles.export(name, builtins()).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    /// Import an exported preset (renamed if the name is taken) and activate it. Accepts both
    /// `craft-keymap` JSON and legacy `effectcraft-shortcuts` documents.
    pub fn import(&mut self, doc: &Value, name: Option<&str>) -> Result<String, String> {
        let base = name.or_else(|| doc.get("name").and_then(Value::as_str)).unwrap_or("Imported");
        let base = if base == DEFAULT_PRESET { "Imported" } else { base };
        let is_craft = doc.get("format").and_then(Value::as_str) == Some("craft-keymap");
        let payload = if is_craft {
            let mut d = doc.clone();
            d["name"] = json!(base);
            d
        } else {
            let ov = doc.get("overrides").ok_or_else(|| "bad shortcut preset: missing overrides".to_string())?;
            if !ov.is_object() && !ov.is_array() {
                return Err("bad shortcut preset: overrides must be a map or list".into());
            }
            let overrides = if ov.is_object() { normalize_override_map(ov) } else { ov.clone() };
            json!({"format": "craft-keymap", "version": 1, "name": base, "overrides": overrides})
        };
        self.profiles.import(&payload.to_string(), builtins()).map_err(|e| format!("bad shortcut preset: {e}"))
    }
}

/// The active preset resolved against every bindable command via [`Keymap`].
#[derive(Clone, Debug)]
pub struct ShortcutTable {
    pub keymap: Keymap,
    first: HashMap<String, String>,
    physical: HashMap<String, Vec<String>>,
}

impl Default for ShortcutTable {
    fn default() -> Self {
        ShortcutTable { keymap: Keymap::new(Vec::new(), scopes(), Platform::current()), first: HashMap::new(), physical: HashMap::new() }
    }
}

impl Deref for ShortcutTable {
    type Target = Keymap;
    fn deref(&self) -> &Keymap {
        &self.keymap
    }
}

impl DerefMut for ShortcutTable {
    fn deref_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }
}

impl ShortcutTable {
    pub fn build(maps: &Keymaps, ui: &[UiCommand]) -> ShortcutTable {
        Self::build_with_scripts(maps, ui, &[])
    }

    /// [`ShortcutTable::build`] with the File ▸ Scripts entries of `scripts`, listed after the
    /// other File ▸ Scripts entries.
    pub fn build_with_scripts(maps: &Keymaps, ui: &[UiCommand], scripts: &[String]) -> ShortcutTable {
        Self::build_for_platform(maps, ui, scripts, Platform::current())
    }

    pub fn build_for_platform(maps: &Keymaps, ui: &[UiCommand], scripts: &[String], platform: Platform) -> ShortcutTable {
        let mut catalog = bindables_for_platform(ui, platform);
        let at = catalog
            .iter()
            .rposition(|b| b.paths.first().is_some_and(|p| p.starts_with(&["File".to_string(), "Scripts".to_string()])))
            .map_or(catalog.len(), |i| i + 1);
        let new: Vec<Bindable> = script_bindables(scripts).into_iter().filter(|s| !catalog.iter().any(|b| b.command == s.command)).collect();
        catalog.splice(at..at, new);
        let mut clean_ov = Overrides::new();
        let mut physical: HashMap<String, Vec<String>> = HashMap::new();
        for (k, texts) in maps.active_overrides().iter() {
            if texts.iter().any(|t| t.starts_with("Physical+")) {
                physical.insert(k.to_string(), texts.to_vec());
            }
            clean_ov.insert(k.to_string(), texts.iter().map(|t| strip_physical(t).to_string()).collect());
        }
        let keymap = Keymap::new(catalog, scopes(), platform).with_overrides(clean_ov);
        let first = keymap
            .catalog()
            .iter()
            .filter_map(|b| {
                let key = b.command.key();
                let trig =
                    physical.get(&key).and_then(|v| v.first().cloned()).or_else(|| keymap.bindings(&b.command).first().map(|k| k.trigger.to_string()))?;
                Some((key, trig))
            })
            .collect();
        ShortcutTable { keymap, first, physical }
    }

    pub fn find(&self, key: &str) -> Option<&Bindable> {
        CommandRef::parse_key(key).and_then(|c| self.keymap.bindable(&c))
    }

    pub fn keys_of(&self, key: &str) -> Vec<String> {
        if let Some(p) = self.physical.get(key) {
            return p.clone();
        }
        CommandRef::parse_key(key).map(|c| self.keymap.bindings(&c).into_iter().map(|b| b.trigger.to_string()).collect()).unwrap_or_default()
    }

    /// First shortcut of a command / menu entry (for menus).
    pub fn shortcut_of(&self, command: &str, params: &Value) -> Option<&str> {
        self.first.get(&binding_key(command, params)).map(String::as_str)
    }

    /// Every active binding: (keys, bindable).
    pub fn bindings(&self) -> Vec<(String, &Bindable)> {
        self.keymap.catalog().iter().flat_map(|b| self.keys_of(&b.command.key()).into_iter().map(move |k| (k, b))).collect()
    }

    /// Other bindables using `keys` in an overlapping scope.
    pub fn conflicts(&self, b: &Bindable, keys: &str) -> Vec<Conflict> {
        let Some(binding) = parse_binding(keys) else { return vec![] };
        let mut km = self.keymap.clone();
        let _ = km.assign(&b.command, binding.clone(), Policy::Keep);
        let want = binding.trigger.effective(km.platform());
        let s_a = binding.scope_or(&b.scope);
        km.conflicts_with(&b.command)
            .into_iter()
            .filter(|c| c.chord == want)
            .filter_map(|c| {
                let other = km.bindable(&c.b.0)?;
                let s_b = c.b.1.scope_or(&other.scope);
                let unrelated_panels = km.scopes().kind(s_a) == ScopeKind::Panel
                    && km.scopes().kind(s_b) == ScopeKind::Panel
                    && s_a != s_b
                    && !km.scopes().shares(s_a.as_str(), s_b.as_str());
                if unrelated_panels {
                    return None;
                }
                Some(Conflict {
                    keys: c.chord.to_string(),
                    key: c.b.0.key(),
                    label: other.label.clone(),
                    scope: s_b.to_string(),
                    overrides: c.kind == ConflictKind::Shadow,
                })
            })
            .collect()
    }

    /// Every pair of commands sharing keys in the same scope, or in two contexts.
    pub fn all_conflicts(&self) -> Vec<(String, Conflict)> {
        self.keymap
            .conflicts()
            .into_iter()
            .filter(|c| c.kind == ConflictKind::Clash && c.chord.chord.key != Key::KeypadMultiply)
            .filter_map(|c| {
                let other = self.keymap.bindable(&c.b.0)?;
                let s_b = c.b.1.scope_or(&other.scope);
                Some((
                    c.a.0.key(),
                    Conflict { keys: c.chord.to_string(), key: c.b.0.key(), label: other.label.clone(), scope: s_b.to_string(), overrides: false },
                ))
            })
            .collect()
    }
}
