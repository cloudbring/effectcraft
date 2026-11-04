//! Edit ▸ Keyboard Shortcuts: the shared `craft-keymap` shortcuts dialog over the engine's
//! catalog and presets (`shortcuts.json`). The dialog edits working copies of the active table's
//! [`Keymap`] and of the [`Profiles`]; OK stores them in the session and saves the file. The
//! `shortcuts.*` engine commands stay the way for agents to do the same.

use std::borrow::Cow;

use craft_keymap::ui::{DialogState, Options, Outcome, ShortcutsDialog, Text};
use effectcraft_engine::shortcuts::{Keymap, Overrides, Profiles, builtins};

use crate::i18n::tr;
use crate::theme::Tokens;
use crate::{Dialog, EffectcraftApp};

/// Dialog state: the widget state and the copies it edits (taken when the dialog first draws).
#[derive(Clone, Debug, Default)]
pub struct EditorState {
    dialog: DialogState,
    work: Option<(Keymap, Profiles)>,
}

struct AppText;

impl Text for AppText {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        tr(source).into()
    }
    fn plural(&self, n: u64, one: &str, other: &str) -> String {
        tr(if n == 1 { one } else { other }).replace("{n}", &n.to_string())
    }
}

pub fn open(app: &mut EffectcraftApp) {
    app.dialog_state.shortcuts = EditorState::default();
    // Scripts copied into the Scripts folder since the table was built are listed too.
    app.session.scripts_changed();
    app.dialog = Some(Dialog::Shortcuts);
}

/// `true` while the dialog is up: it handles Escape itself (cancelling a recording or a prompt
/// first), so the app's dispatcher leaves the key alone.
pub fn recording(app: &EffectcraftApp) -> bool {
    app.dialog == Some(Dialog::Shortcuts)
}

/// Keep `Physical+` keys the dialog (which knows only logical chords) left as they were.
fn keep_physical(old: &Overrides, new: &mut Overrides) {
    for (k, texts) in old.iter().filter(|(_, t)| t.iter().any(|t| t.starts_with("Physical+"))) {
        let plain: Vec<String> = texts.iter().map(|t| t.trim_start_matches("Physical+").to_string()).collect();
        if new.get(k).is_some_and(|n| n == plain.as_slice()) {
            new.insert(k.to_string(), texts.to_vec());
        }
    }
}

fn dialog<'a>(keymap: &'a mut Keymap, sets: &'a mut Profiles, text: &'a AppText, options: &'a Options) -> ShortcutsDialog<'a> {
    ShortcutsDialog::new(keymap, text).sets(sets, builtins()).options(options)
}

pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context, _t: &Tokens) {
    if app.dialog_state.shortcuts.work.is_none() {
        app.dialog_state.shortcuts.work = Some((app.session.shortcuts().keymap.clone(), app.session.keymaps.profiles.clone()));
    }
    let st = &mut app.dialog_state.shortcuts;
    let Some((keymap, sets)) = st.work.as_mut() else { return };
    let options = Options::default().with(|o| {
        o.keyboard = true;
        o.enter_is_ok = true;
    });
    let text = AppText;
    let mut out = dialog(keymap, sets, &text, &options).show(ctx, &mut st.dialog);
    if let Some(Outcome::Import { .. }) = out {
        let picked = app.hooks.pick_files.as_ref().and_then(|f| f(&["json"]).into_iter().next());
        if let Some(path) = picked {
            let res = std::fs::read_to_string(&path).map_err(|e| e.to_string());
            if let Err(e) = res.and_then(|t| dialog(keymap, sets, &text, &options).import_json(&mut st.dialog, &t).map_err(|e| e.to_string())) {
                st.dialog.show_error(e);
            }
        }
        out = None;
    }
    match out {
        Some(Outcome::Export { name, json }) => {
            if let Some(path) = app.hooks.pick_save_file.as_ref().and_then(|f| f(&format!("{name}.json"), "json"))
                && let Err(e) = std::fs::write(&path, json)
            {
                app.ui.status = e.to_string();
            }
        }
        Some(Outcome::Ok) => {
            let mut sets = sets.clone();
            let old = app.session.keymaps.active_overrides().clone();
            let mut new = sets.active_overrides(builtins()).clone();
            keep_physical(&old, &mut new);
            if !Profiles::is_builtin(sets.active_name(builtins()), builtins()) {
                let _ = sets.store(new, builtins(), "Custom");
            }
            app.session.keymaps.profiles = sets;
            app.session.shortcuts_changed();
            app.dialog = None;
        }
        Some(Outcome::Cancel) => app.dialog = None,
        _ => {}
    }
}
