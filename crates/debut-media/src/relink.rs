//! Batch relink (MED-06): find moved or renamed-folder media again by file
//! name in a folder tree. When several files share the name, the one whose
//! parent folders match the old path most closely wins; a tie is left for
//! the user to pick.

use debut_core::MediaId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relink {
    /// One best candidate.
    Found(String),
    /// Equally good candidates: the user decides.
    Ambiguous(Vec<String>),
    /// Nothing with that file name.
    Missing,
}

fn parts(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// How many trailing path components two paths share (the file name counts).
fn shared_tail(a: &[String], b: &[String]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// For each missing `(media, old path)`, the candidate file to relink to.
/// Names compare case-insensitively.
pub fn find(missing: &[(MediaId, String)], candidates: &[String]) -> Vec<(MediaId, Relink)> {
    let cands: Vec<(Vec<String>, &String)> = candidates.iter().map(|c| (parts(c), c)).collect();
    missing
        .iter()
        .map(|(id, old)| {
            let old = parts(old);
            let scored: Vec<(usize, &String)> = cands
                .iter()
                .map(|(p, c)| (shared_tail(&old, p), *c))
                .filter(|(score, _)| *score > 0)
                .collect();
            let best = scored.iter().map(|s| s.0).max().unwrap_or(0);
            let mut top: Vec<String> = scored
                .iter()
                .filter(|s| s.0 == best)
                .map(|s| s.1.clone())
                .collect();
            top.sort();
            top.dedup();
            let r = match top.len() {
                0 => Relink::Missing,
                1 => Relink::Found(top.remove(0)),
                _ => Relink::Ambiguous(top),
            };
            (*id, r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::IdGen;

    #[test]
    fn matches_by_name_preferring_the_same_folders() {
        let mut ids = IdGen::new(1);
        let (a, b, c, d) = (ids.fresh(), ids.fresh(), ids.fresh(), ids.fresh());
        let missing = vec![
            (a, "/old/shoot/day1/A001.MOV".to_string()),
            (b, "/old/shoot/day2/A001.mov".to_string()),
            (c, "/old/music/song.wav".to_string()),
            (d, "/old/gone.mp4".to_string()),
        ];
        let candidates = vec![
            "/new/day1/A001.mov".to_string(),
            "/new/day2/A001.mov".to_string(),
            "/new/x/song.wav".to_string(),
            "/new/y/song.wav".to_string(),
        ];
        let r = find(&missing, &candidates);
        assert_eq!(r[0].1, Relink::Found("/new/day1/A001.mov".into()));
        assert_eq!(r[1].1, Relink::Found("/new/day2/A001.mov".into()));
        assert_eq!(
            r[2].1,
            Relink::Ambiguous(vec!["/new/x/song.wav".into(), "/new/y/song.wav".into()])
        );
        assert_eq!(r[3].1, Relink::Missing);
    }
}
