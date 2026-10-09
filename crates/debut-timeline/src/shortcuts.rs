//! Keyboard shortcuts with NLE presets (TL-12). A keymap binds key chords to
//! editor actions; the UI looks chords up and runs the action, so every key
//! lives in one table per preset instead of scattered through the interface.
//! Presets follow the editors people come from: debut's own, Premiere Pro,
//! Final Cut Pro and Avid Media Composer.

/// Something a shortcut can do. Names are the stable ids the UI dispatches on.
pub const ACTIONS: &[(&str, &str)] = &[
    ("play_pause", "Play / pause"),
    ("shuttle_back", "Shuttle backwards (J)"),
    ("pause", "Stop (K)"),
    ("shuttle_forward", "Shuttle forwards (L)"),
    ("step_back", "Step one frame back"),
    ("step_forward", "Step one frame forward"),
    ("step_back_10", "Step ten frames back"),
    ("step_forward_10", "Step ten frames forward"),
    ("go_to_start", "Go to start"),
    ("add_marker", "Add marker at the playhead"),
    ("blade", "Blade at the playhead"),
    ("ripple_delete", "Ripple delete the selected clip"),
    ("lift", "Lift the selected clip (leave a gap)"),
    ("toggle_snap", "Snapping on / off"),
    ("toggle_linked", "Linked selection on / off"),
    ("undo", "Undo"),
    ("redo", "Redo"),
    ("save", "Save"),
    ("angle_1", "Multicam angle 1"),
    ("angle_2", "Multicam angle 2"),
    ("angle_3", "Multicam angle 3"),
    ("angle_4", "Multicam angle 4"),
    ("angle_5", "Multicam angle 5"),
    ("angle_6", "Multicam angle 6"),
    ("angle_7", "Multicam angle 7"),
    ("angle_8", "Multicam angle 8"),
    ("angle_9", "Multicam angle 9"),
];

/// A key with modifiers. `key` is the `KeyboardEvent.key` value, lowercased
/// for letters ("j", " ", "arrowleft", "delete"). `cmd` is Ctrl, or ⌘ on a Mac.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: String,
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    /// Parse "cmd+shift+z", "space", "shift+arrowleft", "1".
    pub fn parse(text: &str) -> Option<Chord> {
        let mut chord = Chord {
            key: String::new(),
            cmd: false,
            shift: false,
            alt: false,
        };
        for part in text.to_lowercase().split('+') {
            match part {
                "cmd" | "ctrl" => chord.cmd = true,
                "shift" => chord.shift = true,
                "alt" | "option" => chord.alt = true,
                "space" => chord.key = " ".into(),
                "" => return None,
                k => chord.key = k.into(),
            }
        }
        (!chord.key.is_empty()).then_some(chord)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub action: &'static str,
    pub chord: Chord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    pub id: &'static str,
    pub name: &'static str,
    pub bindings: Vec<Binding>,
}

impl Keymap {
    /// The action bound to `chord`, if any.
    pub fn action(&self, chord: &Chord) -> Option<&'static str> {
        self.bindings
            .iter()
            .find(|b| b.chord == *chord)
            .map(|b| b.action)
    }
}

/// Bindings every preset shares: transport on J/K/L and the arrows, angles on
/// the number keys, undo/redo/save on the platform keys.
const COMMON: &[(&str, &str)] = &[
    ("space", "play_pause"),
    ("j", "shuttle_back"),
    ("k", "pause"),
    ("l", "shuttle_forward"),
    ("arrowleft", "step_back"),
    ("arrowright", "step_forward"),
    ("shift+arrowleft", "step_back_10"),
    ("shift+arrowright", "step_forward_10"),
    ("home", "go_to_start"),
    ("cmd+z", "undo"),
    ("cmd+shift+z", "redo"),
    ("cmd+s", "save"),
    ("1", "angle_1"),
    ("2", "angle_2"),
    ("3", "angle_3"),
    ("4", "angle_4"),
    ("5", "angle_5"),
    ("6", "angle_6"),
    ("7", "angle_7"),
    ("8", "angle_8"),
    ("9", "angle_9"),
];

const DEBUT: &[(&str, &str)] = &[
    ("m", "add_marker"),
    ("b", "blade"),
    ("shift+delete", "ripple_delete"),
    ("delete", "lift"),
    ("s", "toggle_snap"),
    ("cmd+l", "toggle_linked"),
];

const PREMIERE: &[(&str, &str)] = &[
    ("m", "add_marker"),
    ("cmd+k", "blade"),
    ("shift+delete", "ripple_delete"),
    ("delete", "lift"),
    ("s", "toggle_snap"),
    ("cmd+l", "toggle_linked"),
];

const FINAL_CUT: &[(&str, &str)] = &[
    ("m", "add_marker"),
    ("cmd+b", "blade"),
    // The magnetic timeline closes up on Delete; Shift+Delete leaves a gap.
    ("delete", "ripple_delete"),
    ("shift+delete", "lift"),
    ("n", "toggle_snap"),
    ("cmd+l", "toggle_linked"),
];

const AVID: &[(&str, &str)] = &[
    ("f5", "add_marker"),
    ("h", "blade"),
    ("x", "ripple_delete"),
    ("z", "lift"),
    ("s", "toggle_snap"),
    ("cmd+l", "toggle_linked"),
];

fn build(id: &'static str, name: &'static str, own: &[(&str, &'static str)]) -> Keymap {
    let bindings = COMMON
        .iter()
        .chain(own)
        .map(|(chord, action)| Binding {
            action,
            chord: Chord::parse(chord).expect("preset chords parse"),
        })
        .collect();
    Keymap { id, name, bindings }
}

/// Every preset, the default first.
pub fn presets() -> Vec<Keymap> {
    vec![
        build("debut", "debut", DEBUT),
        build("premiere", "Premiere Pro", PREMIERE),
        build("final_cut", "Final Cut Pro", FINAL_CUT),
        build("avid", "Avid Media Composer", AVID),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn presets_bind_known_actions_without_conflicts() {
        let known: HashSet<&str> = ACTIONS.iter().map(|a| a.0).collect();
        for map in presets() {
            let mut chords = HashSet::new();
            let mut actions = HashSet::new();
            for b in &map.bindings {
                assert!(
                    known.contains(b.action),
                    "{} binds unknown {}",
                    map.id,
                    b.action
                );
                assert!(
                    chords.insert(b.chord.clone()),
                    "{} binds {:?} twice",
                    map.id,
                    b.chord
                );
                actions.insert(b.action);
            }
            // Every preset covers every action.
            assert_eq!(actions.len(), known.len(), "{} misses actions", map.id);
        }
    }

    #[test]
    fn chords_parse_and_look_up() {
        let fcp = presets().into_iter().find(|m| m.id == "final_cut").unwrap();
        assert_eq!(fcp.action(&Chord::parse("cmd+b").unwrap()), Some("blade"));
        assert_eq!(
            fcp.action(&Chord::parse("Space").unwrap()),
            Some("play_pause")
        );
        assert_eq!(fcp.action(&Chord::parse("b").unwrap()), None);
        let c = Chord::parse("cmd+shift+z").unwrap();
        assert!(c.cmd && c.shift && !c.alt && c.key == "z");
        assert!(Chord::parse("cmd+").is_none());
    }
}
