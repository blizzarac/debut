//! TCP line connections for collaboration: a reader thread turns the socket
//! into a queue of lines, so polling never blocks the UI.

use debut_core::{Error, Result};
use debut_platform::Connection;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

pub struct TcpConnection {
    stream: TcpStream,
    rx: Receiver<String>,
    open: Arc<AtomicBool>,
}

impl TcpConnection {
    pub fn connect(addr: &str) -> Result<Self> {
        let target = addr
            .to_socket_addrs()
            .map_err(|e| Error::InvalidArgument(format!("{addr}: {e}")))?
            .next()
            .ok_or_else(|| Error::InvalidArgument(format!("{addr}: no address")))?;
        let stream = TcpStream::connect_timeout(&target, Duration::from_secs(5))
            .map_err(|e| Error::Other(format!("cannot reach {addr}: {e}")))?;
        stream.set_nodelay(true).ok();
        let reader = stream
            .try_clone()
            .map_err(|e| Error::Other(e.to_string()))?;
        let (tx, rx) = channel();
        let open = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&open);
        std::thread::Builder::new()
            .name("debut-collab-read".into())
            .spawn(move || {
                for line in BufReader::new(reader).lines() {
                    match line {
                        Ok(l) => {
                            if tx.send(l).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                flag.store(false, Ordering::Release);
            })
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { stream, rx, open })
    }
}

impl Connection for TcpConnection {
    fn send(&mut self, line: &str) -> Result<()> {
        let mut data = line.to_string();
        if !data.ends_with('\n') {
            data.push('\n');
        }
        self.stream.write_all(data.as_bytes()).map_err(|e| {
            self.open.store(false, Ordering::Release);
            Error::Other(format!("connection lost: {e}"))
        })
    }

    fn poll(&mut self) -> Vec<String> {
        self.rx.try_iter().collect()
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

impl Drop for TcpConnection {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// The host part of a URL.
fn host_of(url: &str) -> &str {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("");
    }
    authority.split(':').next().unwrap_or("")
}

/// Whether to go through the environment's proxy: not for loopback
/// addresses, nor for hosts `NO_PROXY` / `no_proxy` lists (ureq reads the
/// proxy variables but not that one).
fn use_proxy(url: &str) -> bool {
    let host = host_of(url).to_ascii_lowercase();
    if host == "localhost" || host.starts_with("127.") || host == "::1" {
        return false;
    }
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    !no_proxy.split(',').map(str::trim).any(|p| {
        let p = p.trim_start_matches('.').to_ascii_lowercase();
        p == "*" || (!p.is_empty() && (host == p || host.ends_with(&format!(".{p}"))))
    })
}

/// HTTP(S) through ureq (rustls; `HTTPS_PROXY` and friends are honoured).
pub fn http(req: debut_platform::HttpRequest) -> Result<debut_platform::HttpResponse> {
    use debut_platform::HttpBody;
    use std::io::{Read, Seek, SeekFrom};
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(use_proxy(&req.url))
        .timeout_connect(std::time::Duration::from_secs(15))
        // 3xx are answers: YouTube's resumable uploads reply 308 with no
        // Location to say how much they have.
        .redirects(0)
        .build();
    let mut r = agent.request(&req.method, &req.url);
    for (name, value) in &req.headers {
        r = r.set(name, value);
    }
    let result = match req.body {
        HttpBody::None => r.call(),
        HttpBody::Bytes(b) => r.send_bytes(&b),
        HttpBody::File { path, offset, len } => {
            let mut f =
                std::fs::File::open(&path).map_err(|e| Error::Other(format!("{path}: {e}")))?;
            f.seek(SeekFrom::Start(offset))
                .map_err(|e| Error::Other(e.to_string()))?;
            r.set("Content-Length", &len.to_string()).send(f.take(len))
        }
    };
    let resp = match result {
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
        Err(e) => return Err(Error::Other(format!("{}: {e}", req.url))),
    };
    let status = resp.status();
    let headers = resp
        .headers_names()
        .into_iter()
        .filter_map(|n| {
            let v = resp.header(&n)?.to_string();
            Some((n.to_ascii_lowercase(), v))
        })
        .collect();
    let mut body = Vec::new();
    resp.into_reader()
        .take(64 << 20)
        .read_to_end(&mut body)
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(debut_platform::HttpResponse {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod http_tests {
    use super::*;

    #[test]
    fn loopback_skips_the_proxy() {
        assert_eq!(host_of("http://127.0.0.1:8080/x"), "127.0.0.1");
        assert_eq!(host_of("https://user@example.com/a?b"), "example.com");
        assert_eq!(host_of("http://[::1]:9/"), "::1");
        assert!(!use_proxy("http://localhost:1/"));
        assert!(!use_proxy("http://127.0.0.1:1/"));
        assert!(!use_proxy("http://[::1]:1/"));
    }
}
