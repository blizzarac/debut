//! Bins, smart bins, keywords, ratings and favorites (MED-07, MED-08).

use debut_core::id::{BinId, MediaId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bin {
    pub id: BinId,
    pub name: String,
    pub kind: BinKind,
    pub items: Vec<MediaId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BinKind {
    Manual,
    /// Rule-based membership, re-evaluated on metadata change.
    Smart {
        rules: Vec<SmartRule>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartRule {
    pub field: String,
    pub op: String,
    pub value: String,
}

impl Bin {
    pub fn manual(id: BinId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            kind: BinKind::Manual,
            items: Vec::new(),
        }
    }

    /// A smart bin whose members are the media whose file name contains `needle`
    /// (case-insensitive).
    pub fn name_contains(id: BinId, name: impl Into<String>, needle: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            kind: BinKind::Smart {
                rules: vec![SmartRule {
                    field: "name".into(),
                    op: "contains".into(),
                    value: needle.into(),
                }],
            },
            items: Vec::new(),
        }
    }

    pub fn is_smart(&self) -> bool {
        matches!(self.kind, BinKind::Smart { .. })
    }

    /// Membership: listed for manual bins, every rule true for smart bins.
    pub fn contains(&self, media: &crate::media_ref::MediaRef) -> bool {
        match &self.kind {
            BinKind::Manual => self.items.contains(&media.id),
            BinKind::Smart { rules } => rules.iter().all(|r| r.matches(media)),
        }
    }
}

impl SmartRule {
    pub fn matches(&self, media: &crate::media_ref::MediaRef) -> bool {
        let name = media.path.rsplit('/').next().unwrap_or(&media.path);
        let text = match self.field.as_str() {
            "name" => name.to_string(),
            "path" => media.path.clone(),
            "reel" => media.metadata.reel.clone().unwrap_or_default(),
            "camera" => media.metadata.camera.clone().unwrap_or_default(),
            "audio" => (media.metadata.audio_channels > 0).to_string(),
            _ => return false,
        };
        let (a, b) = (text.to_lowercase(), self.value.to_lowercase());
        match self.op.as_str() {
            "contains" => a.contains(&b),
            "eq" | "is" => a == b,
            "starts" => a.starts_with(&b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_ref::{MediaMetadata, MediaRef};
    use debut_core::IdGen;

    #[test]
    fn manual_bins_list_members_and_smart_bins_match_rules() {
        let mut ids = IdGen::new(2);
        let m = |path: &str, ch: u16, ids: &mut IdGen| MediaRef {
            id: ids.fresh(),
            path: path.into(),
            online: true,
            metadata: MediaMetadata {
                audio_channels: ch,
                ..Default::default()
            },
            proxies: vec![],
        };
        let a = m("/shoot/Interview_A.mov", 2, &mut ids);
        let b = m("/shoot/broll_02.mp4", 0, &mut ids);
        let mut manual = Bin::manual(ids.fresh(), "Selects");
        manual.items.push(b.id);
        assert!(!manual.contains(&a) && manual.contains(&b));
        let smart = Bin::name_contains(ids.fresh(), "Interviews", "interview");
        assert!(smart.contains(&a) && !smart.contains(&b) && smart.is_smart());
        let silent = Bin {
            kind: BinKind::Smart {
                rules: vec![SmartRule {
                    field: "audio".into(),
                    op: "eq".into(),
                    value: "false".into(),
                }],
            },
            ..Bin::manual(ids.fresh(), "Silent")
        };
        assert!(!silent.contains(&a) && silent.contains(&b));
    }
}
