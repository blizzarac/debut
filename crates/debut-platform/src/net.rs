//! Line-based network connections (COL-02): the collaboration client talks to
//! its server through these. Natively a TCP socket; in the browser a
//! WebSocket once the web platform provides one.

use debut_core::Result;

pub trait Connection: Send {
    /// Send one line (a trailing newline is added if missing).
    fn send(&mut self, line: &str) -> Result<()>;
    /// Lines received since the last call, oldest first; never blocks.
    fn poll(&mut self) -> Vec<String>;
    /// False once the other side closed or the link broke.
    fn is_open(&self) -> bool;
}

/// What an HTTP request sends as its body.
#[derive(Clone, Debug, PartialEq)]
pub enum HttpBody {
    None,
    Bytes(Vec<u8>),
    /// `len` bytes of a file from `offset`, streamed (uploads, EXP-09).
    File {
        path: String,
        offset: u64,
        len: u64,
    },
}

/// One HTTP(S) request (uploads EXP-09, telemetry NFR-15).
#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: HttpBody,
}

impl HttpRequest {
    pub fn new(method: &str, url: &str) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers: Vec::new(),
            body: HttpBody::None,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: HttpBody) -> Self {
        self.body = body;
        self
    }
}

/// The answer: any status (4xx/5xx are answers too, not errors).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    /// Names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }
}
