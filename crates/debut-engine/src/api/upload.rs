//! Uploading finished exports (EXP-09), in the background: see
//! `debut_export::upload`. Tokens live only in the running job.

use super::*;
use debut_export::upload::{upload, Destination};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct UploadDto {
    pub id: u64,
    pub file: String,
    /// "folder", "url", "YouTube" or "Vimeo" (never the token or URL).
    pub destination: String,
    pub sent: u64,
    pub total: u64,
    /// "running", "done", "failed" or "cancelled".
    pub state: String,
    /// Where it ended up (a path, URL or video page) once done.
    pub location: Option<String>,
    pub error: Option<String>,
}

pub(crate) struct UploadJob {
    dto: UploadDto,
    cancel: Arc<AtomicBool>,
}

pub(crate) type Uploads = Arc<Mutex<Vec<UploadJob>>>;

impl Session {
    /// Upload `file` to `destination` in the background; returns the job id.
    pub fn upload_start(&mut self, file: String, destination: Destination) -> Result<u64, String> {
        if !self.store.exists(&file) {
            return Err(format!("no file at {file}"));
        }
        self.note_feature(&format!("upload:{}", destination.name()));
        let cancel = Arc::new(AtomicBool::new(false));
        let id = {
            let mut jobs = self.uploads.lock().unwrap_or_else(|e| e.into_inner());
            let id = jobs.len() as u64 + 1;
            jobs.push(UploadJob {
                dto: UploadDto {
                    id,
                    file: file.clone(),
                    destination: destination.name().into(),
                    sent: 0,
                    total: 0,
                    state: "running".into(),
                    location: None,
                    error: None,
                },
                cancel: Arc::clone(&cancel),
            });
            id
        };
        let jobs = Arc::clone(&self.uploads);
        let platform = Arc::clone(&self.platform);
        let update = move |jobs: &Uploads, f: &dyn Fn(&mut UploadDto)| {
            if let Some(j) = jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter_mut()
                .find(|j| j.dto.id == id)
            {
                f(&mut j.dto);
            }
        };
        self.platform
            .spawn(
                "debut-upload",
                Box::new(move || {
                    let result = upload(
                        platform.as_ref(),
                        &file,
                        &destination,
                        &mut |sent, total| {
                            update(&jobs, &|d| {
                                d.sent = sent;
                                d.total = total;
                            })
                        },
                        &|| cancel.load(Ordering::Acquire),
                    );
                    let cancelled = cancel.load(Ordering::Acquire);
                    update(&jobs, &|d| match &result {
                        Ok(u) => {
                            d.state = "done".into();
                            d.location = Some(u.location.clone());
                        }
                        Err(_) if cancelled => d.state = "cancelled".into(),
                        Err(e) => {
                            d.state = "failed".into();
                            d.error = Some(e.to_string());
                        }
                    });
                }),
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn upload_status(&self) -> Vec<UploadDto> {
        self.uploads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|j| j.dto.clone())
            .collect()
    }

    /// Stop an upload after its current chunk.
    pub fn upload_cancel(&self, id: u64) {
        if let Some(j) = self
            .uploads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|j| j.dto.id == id)
        {
            j.cancel.store(true, Ordering::Release);
        }
    }
}
