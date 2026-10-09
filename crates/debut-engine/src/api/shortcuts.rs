//! Keyboard shortcut presets for the UI (TL-12).

use super::*;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ShortcutActionDto {
    pub id: String,
    pub label: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct BindingDto {
    pub action: String,
    /// `KeyboardEvent.key`, lowercased ("j", " ", "arrowleft", "delete").
    pub key: String,
    /// Ctrl, or ⌘ on a Mac.
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct KeymapDto {
    pub id: String,
    pub name: String,
    pub bindings: Vec<BindingDto>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ShortcutsDto {
    pub actions: Vec<ShortcutActionDto>,
    /// Presets, the default first.
    pub keymaps: Vec<KeymapDto>,
}

impl Session {
    /// Every shortcut action and the keymap presets (debut, Premiere Pro,
    /// Final Cut Pro, Avid).
    pub fn shortcuts(&self) -> ShortcutsDto {
        use debut_timeline::shortcuts::{presets, ACTIONS};
        ShortcutsDto {
            actions: ACTIONS
                .iter()
                .map(|(id, label)| ShortcutActionDto {
                    id: (*id).into(),
                    label: (*label).into(),
                })
                .collect(),
            keymaps: presets()
                .into_iter()
                .map(|m| KeymapDto {
                    id: m.id.into(),
                    name: m.name.into(),
                    bindings: m
                        .bindings
                        .into_iter()
                        .map(|b| BindingDto {
                            action: b.action.into(),
                            key: b.chord.key,
                            cmd: b.chord.cmd,
                            shift: b.chord.shift,
                            alt: b.chord.alt,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
