//! OLD shortcut dispatch helpers, vendored for the #188 old-vs-new equivalence test.
//!
//! Source: https://github.com/storytold/effectcraft @ 7870b5916ae0749d497d880989683bf2946d6c8b
//! (merge-base of cloudbring/effectcraft `algo/76-use-craft-keymap` with `origin/main`),
//! file `crates/ui-egui/src/menus/shortcuts.rs`, `parse_shortcut`, `shifted_alias`, `bindings`
//! and `mods_match` (lines 37-130). The only edits: the `cfg!(target_os = "macos")` tests became a
//! `mac` parameter so one test binary can model both the Mac and the Windows profile, and
//! `bindings` takes the OLD `ShortcutTable` instead of a `Session` and a `with_alias` switch
//! (false: leave out the shifted-alias duplicates, to compare the plain binding lists).
//! The engine side (`ShortcutTable`, `Keymaps`, `pick`, `normalize`) is NOT vendored: it is the
//! real old crate, linked as the dev-dependency `effectcraft-engine-old`.

use effectcraft_engine_old::shortcuts::ShortcutTable;
use serde_json::Value;

pub type Binding = (egui::Modifiers, egui::Key, String, Value);

pub fn parse_shortcut(s: &str, mac: bool) -> Option<(egui::Modifiers, egui::Key)> {
    let mut m = egui::Modifiers::NONE;
    let mut key = None;
    let (head, plus) = match s.strip_suffix("++") {
        Some(h) => (h, true),
        None if s == "+" => ("", true),
        None => (s, false),
    };
    let mut parts: Vec<&str> = if head.is_empty() { vec![] } else { head.split('+').collect() };
    if plus {
        parts.push("+");
    }
    for p in parts {
        match p {
            "Cmd" => {
                m.command = true;
                m.mac_cmd = mac;
                if !mac {
                    m.ctrl = true;
                }
            }
            "Physical" => {}
            "Shift" => m.shift = true,
            "Alt" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            k => {
                key = match k {
                    ";" => Some(egui::Key::Semicolon),
                    "'" => Some(egui::Key::Quote),
                    "," => Some(egui::Key::Comma),
                    "." => Some(egui::Key::Period),
                    "/" => Some(egui::Key::Slash),
                    "\\" => Some(egui::Key::Backslash),
                    "=" => Some(egui::Key::Equals),
                    "-" => Some(egui::Key::Minus),
                    "`" => Some(egui::Key::Backtick),
                    "[" => Some(egui::Key::OpenBracket),
                    "]" => Some(egui::Key::CloseBracket),
                    "Num0" => Some(egui::Key::Num0),
                    "Num*" => None,
                    _ => egui::Key::from_name(k),
                }
            }
        }
    }
    key.map(|k| (m, k))
}

fn shifted_alias(k: egui::Key) -> Option<egui::Key> {
    use egui::Key::*;
    Some(match k {
        Equals => Plus,
        Semicolon => Colon,
        OpenBracket => OpenCurlyBracket,
        CloseBracket => CloseCurlyBracket,
        Slash => Questionmark,
        Backslash => Pipe,
        _ => return None,
    })
}

pub fn bindings(table: &ShortcutTable, mac: bool, with_alias: bool) -> Vec<Binding> {
    let mut v: Vec<Binding> = Vec::new();
    for (sc, b) in table.bindings() {
        if sc.starts_with("Physical+") {
            continue;
        }
        if !mac && sc.split('+').any(|p| p == "Ctrl") && sc.split('+').any(|p| p == "Cmd") {
            continue;
        }
        if let Some((m, k)) = parse_shortcut(sc, mac) {
            if with_alias
                && m.shift
                && let Some(alias) = shifted_alias(k)
            {
                v.push((m, alias, b.command.clone(), b.params.clone()));
            }
            v.push((m, k, b.command.clone(), b.params.clone()));
        }
    }
    v.sort_by_key(|(m, ..)| std::cmp::Reverse(m.command as u8 + m.shift as u8 + m.alt as u8 + m.ctrl as u8));
    v
}

pub fn mods_match(want: egui::Modifiers, got: egui::Modifiers, mac: bool) -> bool {
    want.command == got.command && want.shift == got.shift && want.alt == got.alt && (want.ctrl == got.ctrl || got.command && !mac)
}
