//! The plugin-host helper (FX-15, AUD-09, NFR-07). The app starts it and talks
//! to it over stdin/stdout ([`debut_platform::plugin_host::Request`] /
//! `Reply`, one JSON line each, followed by raw f32 samples where announced).
//! Plugins are loaded only here: when one crashes, this process dies, the app
//! reports the error and starts a fresh helper on the next request.

#[cfg(unix)]
mod ffi;
#[cfg(unix)]
mod host;
mod scan;

#[cfg(unix)]
fn main() {
    use std::io::{BufRead, BufReader, BufWriter, Read, Write};
    use std::os::fd::FromRawFd;

    let Some(fd) = ffi::protect_stdout() else {
        eprintln!("debut-plugin-host: cannot set up the protocol pipe");
        std::process::exit(2);
    };
    // SAFETY: `fd` is a fresh duplicate of the original stdout, owned only here.
    let mut out = BufWriter::new(unsafe { std::fs::File::from_raw_fd(fd) });
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut host = host::Host::default();
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let request = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("debut-plugin-host: bad request: {e}");
                return;
            }
        };
        let mut payload = Vec::new();
        if let Some(n) = host::payload_len(&request) {
            payload.resize(n, 0u8);
            if input.read_exact(&mut payload).is_err() {
                return;
            }
        }
        let (reply, data) = host.handle(request, payload);
        let mut text = serde_json::to_string(&reply).expect("replies serialize");
        text.push('\n');
        if out.write_all(text.as_bytes()).is_err()
            || data.as_deref().is_some_and(|d| out.write_all(d).is_err())
            || out.flush().is_err()
        {
            return;
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("debut-plugin-host: plugin hosting needs a Unix-like system for now");
    std::process::exit(2);
}
