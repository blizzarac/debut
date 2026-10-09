//! Presentation surface and external monitoring (PB-05, PB-09, PB-10, PB-11).
//! Desktop: native window + SDI/HDMI cards. Browser: canvas with a WebGPU context.

use debut_core::Result;

pub trait Display: Send {
    fn present(&mut self) -> Result<()>;
    fn size(&self) -> (u32, u32);
    fn supports_hdr(&self) -> bool;
}
