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
