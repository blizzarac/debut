//! Delivery after export (EXP-09): copy a finished file to a folder, PUT it
//! to a pre-signed URL (S3, Google Cloud Storage, Azure SAS), or upload it
//! to YouTube or Vimeo with the user's access token.
//!
//! Large files go up in 8 MiB chunks through resumable protocols (YouTube's
//! resumable upload, Vimeo's tus), so a dropped connection resumes from the
//! last byte the server has instead of starting over. Everything goes
//! through `Platform::http` and the file store; this module makes no OS
//! calls of its own. Getting a token (an OAuth app with the platform) is
//! the user's: debut never stores it.

use debut_core::{Error, Result};
use debut_platform::{HttpBody, HttpRequest, HttpResponse, Platform};
use serde::{Deserialize, Serialize};

/// Bytes per chunk: a multiple of 256 KiB, as YouTube requires.
pub const CHUNK: u64 = 8 << 20;
/// Attempts per chunk before giving up.
const RETRIES: u32 = 5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    /// Copy into a folder (a share, a synced drive, a watch folder).
    Folder { dir: String },
    /// PUT the whole file to a pre-signed URL; `headers` as the signer
    /// requires (e.g. `x-ms-blob-type: BlockBlob` for Azure).
    HttpPut {
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    #[serde(rename = "youtube")]
    YouTube {
        token: String,
        title: String,
        #[serde(default)]
        description: String,
        /// "private", "unlisted" or "public".
        privacy: String,
        /// For tests: instead of https://www.googleapis.com.
        #[serde(default)]
        api_base: Option<String>,
    },
    Vimeo {
        token: String,
        title: String,
        #[serde(default)]
        description: String,
        /// "anybody", "nobody", "unlisted", ...
        privacy: String,
        /// For tests: instead of https://api.vimeo.com.
        #[serde(default)]
        api_base: Option<String>,
    },
}

impl Destination {
    pub fn name(&self) -> &'static str {
        match self {
            Destination::Folder { .. } => "folder",
            Destination::HttpPut { .. } => "url",
            Destination::YouTube { .. } => "YouTube",
            Destination::Vimeo { .. } => "Vimeo",
        }
    }
}

/// Where the file ended up: a path, a URL or a video page.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Uploaded {
    pub location: String,
}

/// Upload `file` to `dest`. `progress(sent, total)` follows the bytes the
/// server confirmed; `cancelled()` is checked between chunks.
pub fn upload(
    platform: &dyn Platform,
    file: &str,
    dest: &Destination,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<Uploaded> {
    let store = platform.file_store();
    let total = store.size(file)?;
    progress(0, total);
    match dest {
        Destination::Folder { dir } => {
            let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
            let to = format!("{}/{name}", dir.trim_end_matches(['/', '\\']));
            store.copy(file, &to)?;
            if store.checksum(&to)? != store.checksum(file)? {
                return Err(Error::Other(format!("the copy at {to} does not match")));
            }
            progress(total, total);
            Ok(Uploaded { location: to })
        }
        Destination::HttpPut { url, headers } => {
            let mut req = HttpRequest::new("PUT", url).body(HttpBody::File {
                path: file.into(),
                offset: 0,
                len: total,
            });
            for (n, v) in headers {
                req = req.header(n, v);
            }
            let resp = with_retries(|| platform.http(req.clone()))?;
            ok(&resp, "the upload URL")?;
            progress(total, total);
            Ok(Uploaded {
                location: url.split('?').next().unwrap_or(url).to_string(),
            })
        }
        Destination::YouTube {
            token,
            title,
            description,
            privacy,
            api_base,
        } => youtube(
            platform,
            file,
            total,
            token,
            title,
            description,
            privacy,
            api_base.as_deref().unwrap_or("https://www.googleapis.com"),
            progress,
            cancelled,
        ),
        Destination::Vimeo {
            token,
            title,
            description,
            privacy,
            api_base,
        } => vimeo(
            platform,
            file,
            total,
            token,
            title,
            description,
            privacy,
            api_base.as_deref().unwrap_or("https://api.vimeo.com"),
            progress,
            cancelled,
        ),
    }
}

fn ok(resp: &HttpResponse, who: &str) -> Result<()> {
    if (200..300).contains(&resp.status) {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{who} answered {}: {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
                .chars()
                .take(300)
                .collect::<String>()
        )))
    }
}

/// Retry transport errors and 5xx answers with a growing pause.
fn with_retries(mut f: impl FnMut() -> Result<HttpResponse>) -> Result<HttpResponse> {
    let mut last = None;
    for attempt in 0..RETRIES {
        match f() {
            Ok(r) if r.status < 500 => return Ok(r),
            Ok(r) => last = Some(Error::Other(format!("server error {}", r.status))),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(200 << attempt));
    }
    Err(last.unwrap_or_else(|| Error::Other("upload failed".into())))
}

fn content_type(file: &str) -> &'static str {
    match file
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        _ => "video/mp4",
    }
}

/// The last byte a YouTube session has, from its `Range: bytes=0-N`.
fn confirmed(resp: &HttpResponse) -> u64 {
    resp.header("range")
        .and_then(|r| r.rsplit('-').next())
        .and_then(|n| n.trim().parse::<u64>().ok())
        .map_or(0, |n| n + 1)
}

#[allow(clippy::too_many_arguments)]
fn youtube(
    platform: &dyn Platform,
    file: &str,
    total: u64,
    token: &str,
    title: &str,
    description: &str,
    privacy: &str,
    base: &str,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<Uploaded> {
    let auth = format!("Bearer {token}");
    let meta = serde_json::json!({
        "snippet": { "title": title, "description": description },
        "status": { "privacyStatus": privacy },
    });
    let start = HttpRequest::new(
        "POST",
        &format!("{base}/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status"),
    )
    .header("Authorization", &auth)
    .header("Content-Type", "application/json; charset=UTF-8")
    .header("X-Upload-Content-Length", &total.to_string())
    .header("X-Upload-Content-Type", content_type(file))
    .body(HttpBody::Bytes(meta.to_string().into_bytes()));
    let resp = with_retries(|| platform.http(start.clone()))?;
    ok(&resp, "YouTube")?;
    let session = resp
        .header("location")
        .ok_or_else(|| Error::Other("YouTube gave no upload session".into()))?
        .to_string();

    let mut sent = 0u64;
    let mut failures = 0;
    loop {
        if cancelled() {
            return Err(Error::Other("upload cancelled".into()));
        }
        let len = CHUNK.min(total - sent);
        let range = if total == 0 {
            "bytes */0".to_string()
        } else {
            format!("bytes {sent}-{}/{total}", sent + len - 1)
        };
        let put = HttpRequest::new("PUT", &session)
            .header("Authorization", &auth)
            .header("Content-Range", &range)
            .body(HttpBody::File {
                path: file.into(),
                offset: sent,
                len,
            });
        match platform.http(put) {
            Ok(r) if r.status == 200 || r.status == 201 => {
                progress(total, total);
                let id = serde_json::from_slice::<serde_json::Value>(&r.body)
                    .ok()
                    .and_then(|v| v["id"].as_str().map(str::to_string))
                    .unwrap_or_default();
                return Ok(Uploaded {
                    location: format!("https://youtu.be/{id}"),
                });
            }
            Ok(r) if r.status == 308 => {
                sent = confirmed(&r);
                failures = 0;
                progress(sent, total);
            }
            Ok(r) if r.status < 500 => ok(&r, "YouTube")?,
            _ => {
                // Ask the session how much it has, then go on from there.
                failures += 1;
                if failures > RETRIES {
                    return Err(Error::Other("YouTube upload kept failing".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(200 << failures));
                let ask = HttpRequest::new("PUT", &session)
                    .header("Authorization", &auth)
                    .header("Content-Range", &format!("bytes */{total}"))
                    .body(HttpBody::Bytes(Vec::new()));
                if let Ok(r) = platform.http(ask) {
                    if r.status == 308 {
                        sent = confirmed(&r);
                        progress(sent, total);
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn vimeo(
    platform: &dyn Platform,
    file: &str,
    total: u64,
    token: &str,
    title: &str,
    description: &str,
    privacy: &str,
    base: &str,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<Uploaded> {
    let create = serde_json::json!({
        "upload": { "approach": "tus", "size": total.to_string() },
        "name": title,
        "description": description,
        "privacy": { "view": privacy },
    });
    let req = HttpRequest::new("POST", &format!("{base}/me/videos"))
        .header("Authorization", &format!("bearer {token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/vnd.vimeo.*+json;version=3.4")
        .body(HttpBody::Bytes(create.to_string().into_bytes()));
    let resp = with_retries(|| platform.http(req.clone()))?;
    ok(&resp, "Vimeo")?;
    let v: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| Error::Other(format!("Vimeo answer: {e}")))?;
    let link = v["upload"]["upload_link"]
        .as_str()
        .ok_or_else(|| Error::Other("Vimeo gave no upload link".into()))?
        .to_string();
    let page = v["link"].as_str().unwrap_or("").to_string();

    // tus: PATCH chunks at the server's offset; HEAD tells the offset.
    let mut sent = 0u64;
    let mut failures = 0;
    while sent < total {
        if cancelled() {
            return Err(Error::Other("upload cancelled".into()));
        }
        let len = CHUNK.min(total - sent);
        let patch = HttpRequest::new("PATCH", &link)
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Offset", &sent.to_string())
            .header("Content-Type", "application/offset+octet-stream")
            .body(HttpBody::File {
                path: file.into(),
                offset: sent,
                len,
            });
        let offset = |r: &HttpResponse| {
            r.header("upload-offset")
                .and_then(|o| o.parse::<u64>().ok())
        };
        match platform.http(patch) {
            Ok(r) if r.status == 204 || r.status == 200 => {
                sent = offset(&r).unwrap_or(sent + len);
                failures = 0;
                progress(sent, total);
            }
            Ok(r) if r.status < 500 && r.status != 409 => ok(&r, "Vimeo")?,
            _ => {
                failures += 1;
                if failures > RETRIES {
                    return Err(Error::Other("Vimeo upload kept failing".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(200 << failures));
                let head = HttpRequest::new("HEAD", &link).header("Tus-Resumable", "1.0.0");
                if let Ok(r) = platform.http(head) {
                    if let Some(o) = offset(&r) {
                        sent = o;
                        progress(sent, total);
                    }
                }
            }
        }
    }
    Ok(Uploaded { location: page })
}

#[cfg(test)]
mod tests;
