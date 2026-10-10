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
