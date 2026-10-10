//! Folder ingest and batch relink (MED-01, MED-02, MED-06, MED-13).

use super::*;

/// Folders deeper than this, or more files than `MAX_FILES`, are not walked.
const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 100_000;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RelinkReport {
    /// (media file name, new path)
    pub relinked: Vec<(String, String)>,
    /// (media file name, equally good candidates): pick one with `relink_media`.
    pub ambiguous: Vec<(String, Vec<String>)>,
    pub not_found: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct IngestReport {
    pub imported: usize,
    /// Already in the project (by path): left alone.
    pub skipped: usize,
    pub copied: usize,
    pub bytes_copied: u64,
    /// (file, why) for files that failed to copy, verify or import.
    pub failed: Vec<(String, String)>,
}

impl Session {
    /// Every file under `dir` (depth- and count-limited), as full paths.
    fn walk(&self, dir: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut stack = vec![(dir.trim_end_matches('/').to_string(), 0usize)];
        while let Some((d, depth)) = stack.pop() {
            let names = self.store.list(&d).map_err(|e| format!("{d}: {e}"))?;
            for name in names {
                let path = format!("{d}/{name}");
                if self.store.is_dir(&path) {
                    if depth < MAX_DEPTH && !name.starts_with('.') {
                        stack.push((path, depth + 1));
                    }
                } else {
                    out.push(path);
                    if out.len() >= MAX_FILES {
                        return Ok(out);
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Look for every offline media file under `dir` by name and relink the
    /// ones with a single best match, in one undo step.
    pub fn relink_folder(&mut self, dir: &str) -> Result<RelinkReport, String> {
        let project = self.project().ok_or("no project open")?;
        let missing: Vec<(MediaId, String)> = project
            .media
            .iter()
            .filter(|m| self.offline.contains(&m.id) || !self.store.exists(&m.path))
            .map(|m| (m.id, m.path.clone()))
            .collect();
        let name = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
        let candidates = self.walk(dir)?;
        let mut report = RelinkReport {
            relinked: Vec::new(),
            ambiguous: Vec::new(),
            not_found: Vec::new(),
        };
        let mut commands = Vec::new();
        for (id, found) in debut_media::relink::find(&missing, &candidates) {
            let old = &missing.iter().find(|m| m.0 == id).expect("asked for").1;
            match found {
                debut_media::relink::Relink::Found(path) => {
                    // Only files that really open count.
                    match self.platform.open_decoder(&path) {
                        Ok(dec) => {
                            if let Some(info) = media_info(dec.as_ref()) {
                                self.probed.insert(id, info);
                            }
                            self.offline.remove(&id);
                            self.forget_waveform(id);
                            if let Some(p) = &mut self.player {
                                p.forget_media(id);
                            }
                            report.relinked.push((name(old), path.clone()));
                            commands.push(Command::SetMediaPath { media: id, path });
                        }
                        Err(_) => report.not_found.push(name(old)),
                    }
                }
                debut_media::relink::Relink::Ambiguous(c) => report.ambiguous.push((name(old), c)),
                debut_media::relink::Relink::Missing => report.not_found.push(name(old)),
            }
        }
        if !commands.is_empty() {
            self.exec(Command::Group(commands))?;
            self.sync_player()?;
        }
        Ok(report)
    }

    /// Import every media file under `source`. With `copy_to`, files are
    /// first copied there (keeping the folder structure) and checked against
    /// the original by checksum; only verified copies are imported. One
    /// undo step for the whole folder.
    pub fn ingest_folder(
        &mut self,
        source: &str,
        copy_to: Option<&str>,
    ) -> Result<IngestReport, String> {
        self.project_mut()?;
        let files: Vec<String> = self
            .walk(source)?
            .into_iter()
            .filter(|f| debut_media::ingest::is_media(f.rsplit('/').next().unwrap_or(f)))
            .collect();
        let mut report = IngestReport {
            imported: 0,
            skipped: 0,
            copied: 0,
            bytes_copied: 0,
            failed: Vec::new(),
        };
        let mut paths = Vec::new();
        match copy_to {
            Some(dest) => {
                for (src, dst) in debut_media::ingest::copy_plan(source, dest, &files) {
                    // An existing, identical copy is reused (a re-run after an
                    // interruption); anything else is copied and verified.
                    let same = self.store.exists(&dst)
                        && self.store.checksum(&dst).ok() == self.store.checksum(&src).ok();
                    if !same {
                        match self.store.copy(&src, &dst) {
                            Ok(n) => {
                                report.copied += 1;
                                report.bytes_copied += n;
                            }
                            Err(e) => {
                                report.failed.push((src, format!("copy failed: {e}")));
                                continue;
                            }
                        }
                        let verified = matches!(
                            (self.store.checksum(&src), self.store.checksum(&dst)),
                            (Ok(a), Ok(b)) if a == b
                        );
                        if !verified {
                            report
                                .failed
                                .push((src, "the copy does not match the original".into()));
                            continue;
                        }
                    }
                    paths.push(dst);
                }
            }
            None => paths = files,
        }
        let known: std::collections::HashSet<String> = self
            .project()
            .map(|p| p.media.iter().map(|m| m.path.clone()).collect())
            .unwrap_or_default();
        let mut commands = Vec::new();
        for path in paths {
            if known.contains(&path) {
                report.skipped += 1;
                continue;
            }
            match self.probe_media(&path) {
                Ok(media) => commands.push(Command::AddMedia(media)),
                Err(e) => report.failed.push((path, e)),
            }
        }
        report.imported = commands.len();
        if !commands.is_empty() {
            self.exec(Command::Group(commands))?;
        }
        Ok(report)
    }
}
