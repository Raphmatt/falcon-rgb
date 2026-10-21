//! Key assignment panel for the selected key.

use eframe::egui::{self, RichText};
use falcon_core::{Function, KeyAssignment, Macro, Profile, layout};

use super::{heading, hint, macro_edit, picker};
use crate::app::{FalconApp, Target};
use crate::theme;
use crate::worker::{Mode, SimAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Default,
    Disabled,
    Key,
    Combo,
    Macro,
    Mouse,
    Media,
    Layer,
    Launch,
    Windows,
}

const CATEGORIES: [(Category, &str); 10] = [
    (Category::Default, "Default"),
    (Category::Disabled, "Disable"),
    (Category::Key, "Single key"),
    (Category::Combo, "Key combo"),
    (Category::Macro, "Macro"),
    (Category::Mouse, "Mouse"),
    (Category::Media, "Media"),
    (Category::Layer, "Layer switch"),
    (Category::Launch, "Launch program"),
    (Category::Windows, "Windows shortcut"),
];

const MOUSE: [Function; 10] = [
    Function::MouseLeft,
    Function::MouseRight,
    Function::MouseMiddle,
    Function::MouseDoubleClick,
    Function::MouseBack,
    Function::MouseForward,
    Function::ScrollUp,
    Function::ScrollDown,
    Function::ScrollLeft,
    Function::ScrollRight,
];
const MEDIA: [Function; 7] = [
    Function::PlayPause,
    Function::PrevTrack,
    Function::NextTrack,
    Function::Stop,
    Function::Mute,
    Function::VolumeDown,
    Function::VolumeUp,
];
const LAYER_FUNCS: [Function; 3] = [
    Function::ProfileCycle,
    Function::ProfileUp,
    Function::ProfileDown,
];
const WINDOWS: [Function; 4] = [
    Function::ShowDesktop,
    Function::Calculator,
    Function::ZoomIn,
    Function::ZoomOut,
];
/// Modifiers offered as checkboxes in the combo editor (left-hand keys).
const MODIFIERS: [(u8, &str); 4] = [
    (0xE0, "Ctrl"),
    (0xE1, "Shift"),
    (0xE2, "Alt / Option"),
    (0xE3, "Win / Cmd"),
];

pub fn category_of(a: &KeyAssignment) -> Category {
    match a {
        KeyAssignment::Default => Category::Default,
        KeyAssignment::Disabled => Category::Disabled,
        KeyAssignment::Key(0xB4..=0xB9) => Category::Layer,
        KeyAssignment::Key(_) => Category::Key,
        KeyAssignment::Combo(_) => Category::Combo,
        KeyAssignment::Macro(_) => Category::Macro,
        KeyAssignment::LaunchProgram(_) => Category::Launch,
        KeyAssignment::Function(f) if MOUSE.contains(f) => Category::Mouse,
        KeyAssignment::Function(f) if MEDIA.contains(f) => Category::Media,
        KeyAssignment::Function(f) if LAYER_FUNCS.contains(f) => Category::Layer,
        KeyAssignment::Function(_) => Category::Windows,
    }
}

/// A sensible first value when the user switches key `k` to `cat`.
pub fn starter(cat: Category, k: usize) -> KeyAssignment {
    match cat {
        Category::Default => KeyAssignment::Default,
        Category::Disabled => KeyAssignment::Disabled,
        Category::Key => KeyAssignment::Key(layout::factory_code(k).unwrap_or(0x04)),
        Category::Combo => KeyAssignment::Combo(vec![0xE0, 0x06]),
        Category::Macro => KeyAssignment::Macro(Macro::default()),
        Category::Mouse => KeyAssignment::Function(Function::MouseLeft),
        Category::Media => KeyAssignment::Function(Function::PlayPause),
        Category::Layer => KeyAssignment::Function(Function::ProfileCycle),
        Category::Launch => KeyAssignment::LaunchProgram(String::new()),
        Category::Windows => KeyAssignment::Function(Function::ShowDesktop),
    }
}

/// Splits a combo into (modifiers, other keys).
pub fn split_combo(codes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    codes.iter().partition(|c| (0xE0..=0xE7).contains(*c))
}

/// Rebuilds a combo: modifiers first, then the key (uploaded as press events in this order).
pub fn join_combo(mods: &[u8], keys: &[u8]) -> Vec<u8> {
    mods.iter().chain(keys).copied().collect()
}

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    let k = match app.target {
        Some(Target::Key(k)) => k,
        _ => {
            heading(ui, "No key selected");
            hint(ui, "Click a key on the keypad to change what it does.");
            return;
        }
    };
    let factory = Profile::factory_label(k).unwrap_or_default();
    heading(ui, &format!("Key {} · {}", k + 1, layout::KEY_LEGENDS[k]));
    hint(ui, &format!("Factory function: {factory}"));
    ui.add_space(6.0);

    let original = app.profile().keys[k].clone();
    let mut a = original.clone();
    let cat = category_of(&a);
    ui.horizontal_wrapped(|ui| {
        for (c, label) in CATEGORIES {
            if ui.selectable_label(cat == c, label).clicked() && cat != c {
                a = starter(c, k);
            }
        }
    });
    ui.separator();

    match category_of(&a) {
        Category::Default => hint(ui, &format!("The key sends its factory code ({factory}).")),
        Category::Disabled => hint(ui, "The key does nothing."),
        Category::Key => {
            let mut code = match a {
                KeyAssignment::Key(c) => Some(c),
                _ => None,
            };
            if picker::palette(ui, &mut code, &mut app.key_filter)
                && let Some(c) = code
            {
                a = KeyAssignment::Key(c);
            }
        }
        Category::Combo => combo_editor(ui, app, &mut a),
        Category::Macro => {
            // The macro editor edits the profile in place (recording needs that).
            if a != original {
                app.profile_mut().keys[k] = a.clone();
            }
            macro_edit::show(ui, app, k);
            a = app.profile().keys[k].clone();
        }
        Category::Mouse => function_list(ui, &mut a, &MOUSE),
        Category::Media => function_list(ui, &mut a, &MEDIA),
        Category::Layer => layer_editor(ui, &mut a),
        Category::Launch => launch_editor(ui, app, &mut a, k),
        Category::Windows => {
            ui.label(
                RichText::new("Windows-specific: these send Windows shortcuts and may do nothing or something else on macOS (Show Desktop = Win+D, i.e. Cmd+D).")
                    .color(theme::WARN)
                    .small(),
            );
            function_list(ui, &mut a, &WINDOWS);
        }
    }

    if let Err(e) = a.validate() {
        ui.add_space(4.0);
        ui.label(RichText::new(format!("Invalid: {e}")).color(theme::ERR));
    }
    if a != original {
        app.profile_mut().keys[k] = a;
    }
}

fn function_list(ui: &mut egui::Ui, a: &mut KeyAssignment, funcs: &[Function]) {
    for &f in funcs {
        let selected = *a == KeyAssignment::Function(f);
        let mut text = f.label().to_owned();
        if f.device_byte().is_none() && f.synthesized_combo().is_none() {
            text.push_str("  (not in the Windows table: key keeps its factory code)");
        }
        if ui.radio(selected, text).clicked() {
            *a = KeyAssignment::Function(f);
        }
    }
}

fn layer_editor(ui: &mut egui::Ui, a: &mut KeyAssignment) {
    hint(ui, "Jump to a layer (key codes handled by the keypad):");
    ui.horizontal_wrapped(|ui| {
        for (i, code) in (0xB4..=0xB9u8).enumerate() {
            let label = if code == 0xB9 {
                "Layer cycle".to_owned()
            } else {
                format!("Layer {}", i + 1)
            };
            if ui
                .selectable_label(*a == KeyAssignment::Key(code), label)
                .clicked()
            {
                *a = KeyAssignment::Key(code);
            }
        }
    });
    ui.add_space(4.0);
    hint(ui, "Or a profile function (Windows function codes):");
    function_list(ui, a, &LAYER_FUNCS);
}

fn combo_editor(ui: &mut egui::Ui, app: &mut FalconApp, a: &mut KeyAssignment) {
    let KeyAssignment::Combo(codes) = a else {
        return;
    };
    let (mut mods, keys) = split_combo(codes);
    ui.horizontal_wrapped(|ui| {
        for (code, name) in MODIFIERS {
            let mut on = mods.contains(&code);
            if ui.checkbox(&mut on, name).changed() {
                if on {
                    mods.push(code);
                    mods.sort_unstable();
                } else {
                    mods.retain(|&c| c != code);
                }
            }
        }
    });
    let extra: Vec<String> = mods
        .iter()
        .filter(|c| !MODIFIERS.iter().any(|(m, _)| m == *c))
        .map(|&c| falcon_core::codes::code_label(c))
        .collect();
    if !extra.is_empty() {
        hint(ui, &format!("Also held: {}", extra.join(", ")));
    }
    let mut key = keys.last().copied();
    ui.add_space(4.0);
    ui.label("Key:");
    picker::palette(ui, &mut key, &mut app.key_filter);
    let keys: Vec<u8> = key.into_iter().collect();
    *codes = join_combo(&mods, &keys);
    ui.label(
        RichText::new(format!("Sends: {}", a.label()))
            .strong()
            .color(theme::ACCENT),
    );
    hint(
        ui,
        "Uploaded as a macro: the keys go down in order and come back up in reverse, 10 ms apart.",
    );
}

fn launch_editor(ui: &mut egui::Ui, app: &mut FalconApp, a: &mut KeyAssignment, k: usize) {
    let KeyAssignment::LaunchProgram(path) = a else {
        return;
    };
    hint(
        ui,
        "Runs on this computer: the keypad reports the key press and Falcon RGB opens the program. The app must be running.",
    );
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(path)
                .hint_text("/Applications/Calculator.app or an app name")
                .desired_width(250.0),
        );
        if ui.button("Browse…").clicked() {
            let mut dlg = rfd::FileDialog::new().set_title("Program to launch");
            if cfg!(target_os = "macos") {
                dlg = dlg.set_directory("/Applications");
            }
            if let Some(p) = dlg.pick_file() {
                *path = p.display().to_string();
            }
        }
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!path.trim().is_empty(), egui::Button::new("Test launch"))
            .clicked()
        {
            match crate::host::launch(path) {
                Ok(()) => app.info(format!("Launched {path}")),
                Err(e) => app.error(format!("Cannot launch {path}: {e}")),
            }
        }
        if app.mode == Mode::Simulated
            && ui
                .button("Simulate key press")
                .on_hover_text("Make the simulated keypad report this key (needs Save first)")
                .clicked()
        {
            app.simulate(SimAction::PressHostKey(k));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_round_trip_through_starters() {
        for (c, _) in CATEGORIES {
            assert_eq!(category_of(&starter(c, 5)), c, "{c:?}");
        }
        assert_eq!(starter(Category::Key, 4), KeyAssignment::Key(0x5F));
        assert_eq!(category_of(&KeyAssignment::Key(0xB6)), Category::Layer);
        assert_eq!(
            category_of(&KeyAssignment::Function(Function::Calculator)),
            Category::Windows
        );
    }

    #[test]
    fn every_function_has_a_category() {
        let listed: Vec<Function> = MOUSE
            .iter()
            .chain(&MEDIA)
            .chain(&LAYER_FUNCS)
            .chain(&WINDOWS)
            .copied()
            .collect();
        for f in Function::ALL {
            assert!(listed.contains(&f), "{f:?}");
        }
    }

    #[test]
    fn combos_keep_modifiers_first() {
        let (m, k) = split_combo(&[0x06, 0xE3, 0xE0]);
        assert_eq!((m.clone(), k.clone()), (vec![0xE3, 0xE0], vec![0x06]));
        assert_eq!(join_combo(&m, &k), vec![0xE3, 0xE0, 0x06]);
    }
}
