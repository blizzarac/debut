//! Native window presentation and SDI/HDMI output (PB-09, PB-11). Pending: the
//! Tauri shell owns the window; this will wrap its wgpu surface.

use debut_core::{Error, Result};
use debut_platform::Display;

pub struct NativeDisplay;

impl Display for NativeDisplay {
    fn present(&mut self) -> Result<()> {
        Err(Error::Unsupported("display not wired yet".into()))
    }
    fn size(&self) -> (u32, u32) {
        (0, 0)
    }
    fn supports_hdr(&self) -> bool {
        false
    }
}
