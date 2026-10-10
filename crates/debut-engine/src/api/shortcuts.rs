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

/// A key chord as the UI sends it.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct ChordDto {
    pub key: String,
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

/// The user's own keys for one action; empty unbinds it.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct KeyOverrideDto {
    pub action: String,
    pub keys: Vec<ChordDto>,
}

fn keymap_dto(m: debut_timeline::shortcuts::Keymap) -> KeymapDto {
    KeymapDto {
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
    }
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
            keymaps: presets().into_iter().map(keymap_dto).collect(),
        }
    }

    /// Preset `id` with the user's own keys on top (TL-12): overridden actions
    /// get exactly their keys, which are taken away from any other action.
    pub fn resolve_keymap(
        &self,
        id: &str,
        overrides: Vec<KeyOverrideDto>,
    ) -> Result<KeymapDto, String> {
        use debut_timeline::shortcuts::{presets, Chord};
        let base = presets()
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("no keymap {id}"))?;
        let overrides: Vec<(String, Vec<Chord>)> = overrides
            .into_iter()
            .map(|o| {
                let keys = o
                    .keys
                    .into_iter()
                    .filter(|k| !k.key.is_empty())
                    .map(|k| Chord {
                        key: k.key.to_lowercase(),
                        cmd: k.cmd,
                        shift: k.shift,
                        alt: k.alt,
                    })
                    .collect();
                (o.action, keys)
            })
            .collect();
        Ok(keymap_dto(base.with_overrides(&overrides)))
    }
}
