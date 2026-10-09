//! Markers on clips and timelines (MED-07, TL-10, COL-04).

use debut_core::id::MarkerId;
use debut_core::{FrameRate, Rational, Timecode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    /// Sequence time for timeline markers, clip-local time for clip markers.
    pub at: Rational,
    pub duration: Rational,
    pub color: [u8; 3],
    pub note: String,
    /// Set when the marker was imported from a review comment (COL-04).
    pub resolved: Option<bool>,
}

impl Marker {
    pub fn new(id: MarkerId, at: Rational, note: impl Into<String>) -> Self {
        Self {
            id,
            at,
            duration: Rational::ZERO,
            color: [59, 130, 246],
            note: note.into(),
            resolved: None,
        }
    }

    pub fn end(&self) -> Rational {
        self.at + self.duration
    }
}

/// One line of an exported marker list (TL-10): timecode in, out, colour, note.
pub fn marker_list(markers: &[(Rational, &Marker)], rate: FrameRate) -> String {
    let mut rows: Vec<&(Rational, &Marker)> = markers.iter().collect();
    rows.sort_by_key(|r| r.0);
    let tc = |t: Rational| {
        Timecode::from_frames(rate.time_to_frame(t), rate, rate.is_fractional()).to_string()
    };
    let mut out = String::from("in\tout\tcolor\tnote\n");
    for (at, m) in rows {
        let note = m.note.replace(['\t', '\n'], " ");
        out.push_str(&format!(
            "{}\t{}\t#{:02x}{:02x}{:02x}\t{}\n",
            tc(*at),
            tc(*at + m.duration),
            m.color[0],
            m.color[1],
            m.color[2],
            note
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::IdGen;

    #[test]
    fn marker_list_is_sorted_tab_separated_timecode() {
        let mut ids = IdGen::new(1);
        let a = Marker {
            duration: Rational::from_int(2),
            note: "tab\there".into(),
            ..Marker::new(ids.fresh(), Rational::from_int(5), "")
        };
        let b = Marker::new(ids.fresh(), Rational::from_int(1), "first");
        let text = marker_list(&[(a.at, &a), (b.at, &b)], FrameRate::FPS_25);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "in\tout\tcolor\tnote");
        assert_eq!(lines[1], "00:00:01:00\t00:00:01:00\t#3b82f6\tfirst");
        assert_eq!(lines[2], "00:00:05:00\t00:00:07:00\t#3b82f6\ttab here");
    }
}
