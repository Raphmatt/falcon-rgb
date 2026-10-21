//! Key-code pickers: the single-key palette (device key map) and macro event codes.

use eframe::egui::{self, RichText};
use falcon_core::codes::{self, KEY_NAMES, MACRO_MOUSE_CODES, SINGLE_KEY_PALETTE};

/// Palette group of a single-key code, in display order.
fn group_of(code: u8) -> usize {
    match code {
        0xFA | 0xBB | 0xBC => 0,
        0xB4..=0xB9 => 1,
        0xA4..=0xAB => 2,
        0xBE | 0xBF | 0xC0 | 0xC3 | 0xC4 => 3,
        0xE0..=0xE7 => 5,
        _ => 4,
    }
}

const GROUPS: [&str; 6] = [
    "Special",
    "Layers",
    "Media",
    "Mouse buttons",
    "Keyboard",
    "Modifiers",
];

fn matches(filter: &str, code: u8, label: &str, name: &str) -> bool {
    let f = filter.trim().to_ascii_lowercase();
    f.is_empty()
        || label.to_ascii_lowercase().contains(&f)
        || name.to_ascii_lowercase().contains(&f)
        || format!("{code:02x}") == f.trim_start_matches("0x")
}

/// Searchable single-key palette (all codes the Windows UI offers, minus "Disable").
/// Returns true when `current` changed.
pub fn palette(ui: &mut egui::Ui, current: &mut Option<u8>, filter: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(egui::TextEdit::singleline(filter).hint_text("e.g. F13, layer, volume, 0x2c"));
        if !filter.is_empty() && ui.small_button("x").clicked() {
            filter.clear();
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("palette")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (g, title) in GROUPS.iter().enumerate() {
                let codes: Vec<u8> = SINGLE_KEY_PALETTE[1..]
                    .iter()
                    .copied()
                    .filter(|&c| group_of(c) == g)
                    .filter(|&c| {
                        let e = codes::key_name(c);
                        matches(
                            filter,
                            c,
                            e.map_or("", |e| e.label),
                            e.map_or("", |e| e.name),
                        )
                    })
                    .collect();
                if codes.is_empty() {
                    continue;
                }
                ui.label(RichText::new(*title).small().weak());
                ui.horizontal_wrapped(|ui| {
                    for c in codes {
                        let resp = ui
                            .selectable_label(*current == Some(c), codes::code_label(c))
                            .on_hover_text(format!("{} ({c:#04x})", codes::code_name(c)));
                        if resp.clicked() {
                            *current = Some(c);
                            changed = true;
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });
    changed
}

/// Codes offered inside macros: standard HID usages plus the macro mouse buttons. Vendor codes
/// are left out because their meaning inside a macro is unknown.
pub fn macro_codes() -> impl Iterator<Item = u8> {
    KEY_NAMES
        .iter()
        .map(|e| e.code)
        .filter(|&c| c < 0xA4 || (0xE0..=0xE7).contains(&c))
        .chain(MACRO_MOUSE_CODES.iter().map(|e| e.code))
}

/// Label of a code inside a macro (`B7..B9` are mouse buttons there).
pub fn macro_label(code: u8) -> String {
    MACRO_MOUSE_CODES
        .iter()
        .find(|e| e.code == code)
        .map_or_else(|| codes::code_label(code), |e| e.label.to_owned())
}

/// Compact drop-down for one macro event's code.
pub fn macro_code_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    code: &mut u8,
) -> bool {
    let before = *code;
    egui::ComboBox::from_id_salt(id)
        .selected_text(macro_label(*code))
        .width(150.0)
        .height(320.0)
        .show_ui(ui, |ui| {
            for c in macro_codes() {
                ui.selectable_value(code, c, macro_label(c));
            }
        });
    *code != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_code_is_in_a_group() {
        for &c in &SINGLE_KEY_PALETTE[1..] {
            assert!(group_of(c) < GROUPS.len());
        }
        assert_eq!(group_of(0xB6), 1);
        assert_eq!(group_of(0x04), 4);
    }

    #[test]
    fn macro_codes_have_no_vendor_codes_except_mouse() {
        let all: Vec<u8> = macro_codes().collect();
        assert!(all.contains(&0x04) && all.contains(&0xE3));
        assert!(all.contains(&0xB7), "mouse left inside macros");
        assert!(!all.contains(&0xBB) && !all.contains(&0xFA));
        assert_eq!(macro_label(0xB7), "Mouse Left (macro)");
        assert_eq!(macro_label(0x04), "A");
    }

    #[test]
    fn search_matches_label_name_or_hex() {
        assert!(matches("", 0x04, "A", "A"));
        assert!(matches("vol", 0xAB, "Volume +", "MEDIA_VOLUP"));
        assert!(matches("0x2c", 0x2C, "Space", "SPACE"));
        assert!(matches("2C", 0x2C, "Space", "SPACE"));
        assert!(!matches("zzz", 0x04, "A", "A"));
    }
}
