//! Timeline and clip markers (TL-10).

use super::*;

#[derive(Serialize, Debug)]
pub struct MarkerDto {
    pub id: String,
    /// Absolute sequence time, also for clip markers.
    pub at: f64,
    pub duration: f64,
    pub color: [u8; 3],
    pub note: String,
    /// `None` for a timeline marker, else the owning clip.
    pub clip: Option<String>,
}

#[derive(Deserialize)]
pub struct MarkerEdit {
    pub note: Option<String>,
    pub color: Option<[u8; 3]>,
    pub duration: Option<f64>,
    pub at: Option<f64>,
}

impl Session {
    /// Timeline markers plus every clip marker, in sequence time.
    pub fn markers(&self) -> Result<Vec<MarkerDto>, String> {
        let seq = self.first_sequence()?;
        let mut out: Vec<MarkerDto> = seq
            .markers
            .iter()
            .map(|m| MarkerDto {
                id: id_str(m.id.0),
                at: secs(m.at),
                duration: secs(m.duration),
                color: m.color,
                note: m.note.clone(),
                clip: None,
            })
            .collect();
        for t in &seq.tracks {
            for c in &t.clips {
                for m in &c.markers {
                    out.push(MarkerDto {
                        id: id_str(m.id.0),
                        at: secs(c.timeline_in + m.at),
                        duration: secs(m.duration),
                        color: m.color,
                        note: m.note.clone(),
                        clip: Some(id_str(c.id.0)),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(out)
    }

    pub(crate) fn marker_target(
        &self,
        id: MarkerId,
    ) -> Result<(MarkerTarget, Marker, Rational), String> {
        let seq = self.first_sequence()?;
        if let Some(m) = seq.markers.iter().find(|m| m.id == id) {
            return Ok((
                MarkerTarget {
                    sequence: seq.id,
                    clip: None,
                },
                m.clone(),
                Rational::ZERO,
            ));
        }
        for t in &seq.tracks {
            for c in &t.clips {
                if let Some(m) = c.markers.iter().find(|m| m.id == id) {
                    return Ok((
                        MarkerTarget {
                            sequence: seq.id,
                            clip: Some((t.id, c.id)),
                        },
                        m.clone(),
                        c.timeline_in,
                    ));
                }
            }
        }
        Err("marker not found".into())
    }

    /// Add a timeline marker, or a clip marker when `clip` is given, at `at` seconds.
    pub fn add_marker(
        &mut self,
        at: f64,
        note: String,
        clip: Option<String>,
    ) -> Result<String, String> {
        let seq = self.first_sequence()?;
        let (seq_id, fr) = (seq.id, seq.frame_rate);
        let t = frames_of(at, fr);
        let (target, local) = match clip {
            None => (
                MarkerTarget {
                    sequence: seq_id,
                    clip: None,
                },
                t,
            ),
            Some(cid) => {
                let cid = ClipId(parse_id(&cid)?);
                let (track, c) = seq
                    .tracks
                    .iter()
                    .find_map(|tr| tr.clip(cid).map(|c| (tr.id, c)))
                    .ok_or("clip not found")?;
                (
                    MarkerTarget {
                        sequence: seq_id,
                        clip: Some((track, cid)),
                    },
                    t - c.timeline_in,
                )
            }
        };
        let marker = Marker::new(self.ids.fresh(), local, note);
        let id = id_str(marker.id.0);
        self.exec(Command::AddMarker { target, marker })?;
        Ok(id)
    }

    pub fn update_marker(&mut self, id: &str, edit: MarkerEdit) -> Result<(), String> {
        let fr = self.first_sequence()?.frame_rate;
        let (target, mut m, base) = self.marker_target(MarkerId(parse_id(id)?))?;
        if let Some(n) = edit.note {
            m.note = n;
        }
        if let Some(c) = edit.color {
            m.color = c;
        }
        if let Some(d) = edit.duration {
            m.duration = frames_of(d.max(0.0), fr);
        }
        if let Some(a) = edit.at {
            m.at = frames_of(a, fr) - base;
        }
        self.exec(Command::UpdateMarker { target, marker: m })
    }

    pub fn remove_marker(&mut self, id: &str) -> Result<(), String> {
        let (target, m, _) = self.marker_target(MarkerId(parse_id(id)?))?;
        self.exec(Command::RemoveMarker { target, id: m.id })
    }

    /// Write the marker list (tab-separated timecodes) to `path`.
    pub fn export_markers(&self, path: &str) -> Result<usize, String> {
        let seq = self.first_sequence()?;
        let mut rows: Vec<(Rational, &Marker)> = seq.markers.iter().map(|m| (m.at, m)).collect();
        for t in &seq.tracks {
            for c in &t.clips {
                rows.extend(c.markers.iter().map(|m| (c.timeline_in + m.at, m)));
            }
        }
        let text = debut_project::marker_list(&rows, seq.frame_rate);
        self.store
            .write(path, text.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(rows.len())
    }
}
