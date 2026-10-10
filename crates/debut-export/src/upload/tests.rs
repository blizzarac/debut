#![allow(clippy::disallowed_methods, clippy::disallowed_types)] // tests use the OS directly

use super::*;
use debut_platform_native::NativePlatform;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// What the mock services received.
#[derive(Default)]
struct State {
    /// Uploaded bytes per session / object.
    files: HashMap<String, Vec<u8>>,
    /// Requests seen, as "METHOD path".
    log: Vec<String>,
    /// Metadata posted when a video was created.
    meta: Vec<serde_json::Value>,
    /// Fail (500) the first chunk request after this many bytes, keeping
    /// only half of its body: a connection that died mid-chunk.
    fail_after: Option<u64>,
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &[u8]) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, state: Arc<Mutex<State>>, base: String) {
    let Some(req) = read_request(&mut stream) else {
        return;
    };
    let mut st = state.lock().unwrap();
    st.log.push(format!(
        "{} {}",
        req.method,
        req.path.split('?').next().unwrap()
    ));
    let path = req.path.split('?').next().unwrap().to_string();
    // A chunk request that "dies" halfway: keep half, answer 500.
    let chunk = |st: &mut State, key: &str, body: &[u8]| -> bool {
        let have = st.files.get(key).map_or(0, Vec::len) as u64;
        if let Some(after) = st.fail_after {
            if have >= after && !body.is_empty() {
                st.fail_after = None;
                st.files
                    .entry(key.into())
                    .or_default()
                    .extend_from_slice(&body[..body.len() / 2]);
                return false;
            }
        }
        st.files
            .entry(key.into())
            .or_default()
            .extend_from_slice(body);
        true
    };
    match (req.method.as_str(), path.as_str()) {
        // YouTube resumable upload.
        ("POST", "/upload/youtube/v3/videos") => {
            assert_eq!(req.headers["authorization"], "Bearer yt-token");
            let total: u64 = req.headers["x-upload-content-length"].parse().unwrap();
            st.meta.push(serde_json::from_slice(&req.body).unwrap());
            st.files.insert("yt".into(), Vec::new());
            st.files
                .insert("yt-total".into(), total.to_string().into_bytes());
            respond(
                &mut stream,
                "200 OK",
                &[("Location", format!("{base}/yt-session"))],
                b"",
            );
        }
        ("PUT", "/yt-session") => {
            let total: u64 = String::from_utf8(st.files["yt-total"].clone())
                .unwrap()
                .parse()
                .unwrap();
            let range = req.headers["content-range"].clone();
            if !range.starts_with("bytes */") {
                let start: u64 = range[6..].split('-').next().unwrap().parse().unwrap();
                let have = st.files["yt"].len() as u64;
                assert_eq!(start, have, "chunks must continue where the server is");
                if !chunk(&mut st, "yt", &req.body) {
                    respond(&mut stream, "500 Internal Server Error", &[], b"");
                    return;
                }
            }
            let have = st.files["yt"].len() as u64;
            if have == total {
                respond(&mut stream, "201 Created", &[], br#"{"id":"abc123"}"#);
            } else {
                let range = if have == 0 {
                    vec![]
                } else {
                    vec![("Range", format!("bytes=0-{}", have - 1))]
                };
                respond(&mut stream, "308 Resume Incomplete", &range, b"");
            }
        }
        // Vimeo: create the video, then tus.
        ("POST", "/me/videos") => {
            assert_eq!(req.headers["authorization"], "bearer vm-token");
            let v: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(v["upload"]["approach"], "tus");
            st.meta.push(v);
            st.files.insert("vm".into(), Vec::new());
            let body = serde_json::json!({
                "link": "https://vimeo.com/42",
                "upload": { "upload_link": format!("{base}/tus/42") },
            });
            respond(&mut stream, "200 OK", &[], body.to_string().as_bytes());
        }
        ("PATCH", "/tus/42") => {
            let offset: u64 = req.headers["upload-offset"].parse().unwrap();
            let have = st.files["vm"].len() as u64;
            if offset != have {
                respond(&mut stream, "409 Conflict", &[], b"");
                return;
            }
            if !chunk(&mut st, "vm", &req.body) {
                respond(&mut stream, "500 Internal Server Error", &[], b"");
                return;
            }
            let have = st.files["vm"].len();
            respond(
                &mut stream,
                "204 No Content",
                &[("Upload-Offset", have.to_string())],
                b"",
            );
        }
        ("HEAD", "/tus/42") => {
            let have = st.files["vm"].len();
            respond(
                &mut stream,
                "200 OK",
                &[("Upload-Offset", have.to_string())],
                b"",
            );
        }
        // Pre-signed PUT.
        ("PUT", "/bucket/out.mp4") => {
            assert_eq!(req.headers["x-ms-blob-type"], "BlockBlob");
            st.files.insert("put".into(), req.body.clone());
            respond(&mut stream, "200 OK", &[], b"");
        }
        _ => respond(&mut stream, "404 Not Found", &[], b""),
    }
}

fn server() -> (String, Arc<Mutex<State>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let state = Arc::new(Mutex::new(State::default()));
    let (s, b) = (Arc::clone(&state), base.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (s, b) = (Arc::clone(&s), b.clone());
            std::thread::spawn(move || handle(stream, s, b));
        }
    });
    (base, state)
}

/// A file of 2.5 chunks with recognisable content.
fn sample(name: &str) -> (std::path::PathBuf, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("debut-upload-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.mp4");
    let data: Vec<u8> = (0..(CHUNK * 5 / 2) as usize)
        .map(|i| (i * 7 % 251) as u8)
        .collect();
    std::fs::write(&path, &data).unwrap();
    (path, data)
}

fn run(dest: &Destination, file: &std::path::Path) -> (Result<Uploaded>, Vec<(u64, u64)>) {
    let platform = NativePlatform::new();
    let mut seen = Vec::new();
    let r = upload(
        &platform,
        file.to_str().unwrap(),
        dest,
        &mut |a, b| seen.push((a, b)),
        &|| false,
    );
    (r, seen)
}

#[test]
fn youtube_resumable_upload_survives_a_dropped_chunk() {
    let (base, state) = server();
    state.lock().unwrap().fail_after = Some(CHUNK);
    let (file, data) = sample("yt");
    let dest = Destination::YouTube {
        token: "yt-token".into(),
        title: "Cut 3".into(),
        description: "rough".into(),
        privacy: "unlisted".into(),
        api_base: Some(base),
    };
    let (r, seen) = run(&dest, &file);
    assert_eq!(r.unwrap().location, "https://youtu.be/abc123");
    let st = state.lock().unwrap();
    assert!(
        st.files["yt"] == data,
        "the server has the whole file, in order"
    );
    assert_eq!(st.meta[0]["snippet"]["title"], "Cut 3");
    assert_eq!(st.meta[0]["status"]["privacyStatus"], "unlisted");
    // Chunk 1, chunk 2 (dies halfway), the `bytes */total` status query,
    // then the rest from where the server was (12 MiB) in one chunk.
    let puts = st.log.iter().filter(|l| *l == "PUT /yt-session").count();
    assert_eq!(puts, 4, "{:?}", st.log);
    assert_eq!(
        *seen.last().unwrap(),
        (data.len() as u64, data.len() as u64)
    );
    assert!(
        seen.windows(2).any(|w| w[1].0 < w[0].0 + CHUNK),
        "progress followed the server"
    );
}

#[test]
fn vimeo_tus_upload_survives_a_dropped_chunk() {
    let (base, state) = server();
    state.lock().unwrap().fail_after = Some(CHUNK);
    let (file, data) = sample("vm");
    let dest = Destination::Vimeo {
        token: "vm-token".into(),
        title: "Cut 3".into(),
        description: String::new(),
        privacy: "nobody".into(),
        api_base: Some(base),
    };
    let (r, _) = run(&dest, &file);
    assert_eq!(r.unwrap().location, "https://vimeo.com/42");
    let st = state.lock().unwrap();
    assert!(st.files["vm"] == data);
    assert!(
        st.log.contains(&"HEAD /tus/42".to_string()),
        "asked for the offset: {:?}",
        st.log
    );
    assert_eq!(st.meta[0]["upload"]["size"], data.len().to_string());
}

#[test]
fn presigned_put_and_folder_copy() {
    let (base, state) = server();
    let (file, data) = sample("put");
    let dest = Destination::HttpPut {
        url: format!("{base}/bucket/out.mp4?sig=secret"),
        headers: vec![("x-ms-blob-type".into(), "BlockBlob".into())],
    };
    let (r, _) = run(&dest, &file);
    assert_eq!(
        r.unwrap().location,
        format!("{base}/bucket/out.mp4"),
        "no signature in the result"
    );
    assert!(state.lock().unwrap().files["put"] == data);

    let target = file.parent().unwrap().join("delivered");
    let (r, seen) = run(
        &Destination::Folder {
            dir: target.to_string_lossy().into_owned(),
        },
        &file,
    );
    let loc = r.unwrap().location;
    assert!(std::fs::read(&loc).unwrap() == data);
    assert_eq!(
        *seen.last().unwrap(),
        (data.len() as u64, data.len() as u64)
    );

    // A refused upload is an error with the server's words.
    let (r, _) = run(
        &Destination::HttpPut {
            url: format!("{base}/nowhere"),
            headers: Vec::new(),
        },
        &file,
    );
    assert!(r.unwrap_err().to_string().contains("404"));
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}
