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
        let value = self.value.to_lowercase();
        let text_op = |a: &str| match self.op.as_str() {
            "contains" => a.contains(&value),
            "eq" | "is" => a == value,
            "starts" => a.starts_with(&value),
            _ => false,
        };
        match self.field.as_str() {
            "name" => text_op(&name.to_lowercase()),
            "path" => text_op(&media.path.to_lowercase()),
            "reel" => text_op(
                &media
                    .metadata
                    .reel
                    .clone()
                    .unwrap_or_default()
                    .to_lowercase(),
            ),
            "camera" => text_op(
                &media
                    .metadata
                    .camera
                    .clone()
                    .unwrap_or_default()
                    .to_lowercase(),
            ),
            "audio" => text_op(&(media.metadata.audio_channels > 0).to_string()),
            // Any keyword may satisfy the rule.
            "keyword" => media.keywords.iter().any(|k| text_op(&k.to_lowercase())),
            "rating" => {
                let Ok(n) = value.trim().parse::<u8>() else {
                    return false;
                };
                match self.op.as_str() {
                    "gte" | ">=" => media.rating >= n,
                    "eq" | "is" | "==" => media.rating == n,
                    "lte" | "<=" => media.rating <= n,
                    _ => false,
                }
            }
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
            keywords: vec![],
            rating: 0,
        };
        let mut a = m("/shoot/Interview_A.mov", 2, &mut ids);
        let b = m("/shoot/broll_02.mp4", 0, &mut ids);
        a.keywords = vec!["CEO".into(), "wide".into()];
        a.rating = 4;
        let rule = |field: &str, op: &str, value: &str| SmartRule {
            field: field.into(),
            op: op.into(),
            value: value.into(),
        };
        assert!(
            rule("keyword", "eq", "ceo").matches(&a) && !rule("keyword", "eq", "ceo").matches(&b)
        );
        assert!(rule("rating", "gte", "4").matches(&a) && !rule("rating", "gte", "5").matches(&a));
        assert!(!rule("rating", "gte", "x").matches(&a));
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
