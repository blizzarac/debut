//! Presenting program output straight to a window (PB-05, PB-09): the
//! rendered texture is fitted into the window's surface and run through the
//! output transform there, with no read-back to the CPU and no copy through
//! the UI. A full-screen program monitor on a second display uses this.

use crate::backend::{Backend, Transform2D};
use crate::color::Transfer;
use crate::gpu::{GpuBackend, GpuImage, Pass};

/// A native window to present into: anything with raw window and display
/// handles (a Tauri or winit window).
pub trait WindowSource:
    wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static
{
}

impl<T> WindowSource for T where
    T: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static
{
}

/// A window surface configured for one GPU backend.
pub struct SurfaceViewer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pass: Pass,
}

impl SurfaceViewer {
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }
}

/// Where a `src`-sized picture sits, fitted and centred, in a `dst` canvas.
pub fn fit(src: (u32, u32), dst: (u32, u32)) -> Transform2D {
    let s = (dst.0 as f32 / src.0.max(1) as f32).min(dst.1 as f32 / src.1.max(1) as f32);
    Transform2D::from_srt(src, dst, (s, s), 0.0, (0.0, 0.0))
}

impl GpuBackend {
    /// A surface on `window`, `w x h` pixels, presenting with vsync.
    pub fn create_viewer(
        &self,
        window: impl WindowSource,
        w: u32,
        h: u32,
    ) -> Result<SurfaceViewer, String> {
        let target: Box<dyn wgpu::WindowHandle> = Box::new(window);
        let surface = self
            .instance
            .create_surface(target)
            .map_err(|e| format!("cannot create a surface on the window: {e}"))?;
        let caps = surface.get_capabilities(&self.adapter);
        if caps.formats.is_empty() {
            return Err("the GPU cannot present to this window".into());
        }
        // The output pass encodes the transfer function itself, so prefer a
        // plain (non-sRGB) 8-bit format.
        let format = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ]
        .into_iter()
        .find(|f| caps.formats.contains(f))
        .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: w.max(1),
            height: h.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&self.device, &config);
        let pass = Self::pass_to(
            &self.device,
            "present",
            crate::gpu::COLOR,
            crate::gpu::OUTPUT,
            1,
            false,
            16,
            format,
        );
        Ok(SurfaceViewer {
            surface,
            config,
            pass,
        })
    }

    pub fn resize_viewer(&self, viewer: &mut SurfaceViewer, w: u32, h: u32) {
        viewer.config.width = w.max(1);
        viewer.config.height = h.max(1);
        viewer.surface.configure(&self.device, &viewer.config);
    }

    /// Show `img` in the window, fitted and letterboxed in black, encoded
    /// with `transfer`.
    pub fn present(
        &mut self,
        img: &GpuImage,
        viewer: &mut SurfaceViewer,
        transfer: Transfer,
    ) -> Result<(), String> {
        let (w, h) = viewer.size();
        let fitted = self.transform(img, &fit((img.w, img.h), (w, h)), w, h);
        let frame = match viewer.surface.get_current_texture() {
            Ok(f) => f,
            // The window changed size or was lost: configure again and retry once.
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                viewer.surface.configure(&self.device, &viewer.config);
                viewer
                    .surface
                    .get_current_texture()
                    .map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let fitted = self.premultiplied(&fitted);
        let p = [transfer as u32, 0, 0, 0];
        self.draw(
            &viewer.pass,
            bytemuck::bytes_of(&p),
            &[&fitted],
            None,
            &view,
            "present",
        );
        frame.present();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_centred() {
        // 16:9 into a square: full width, centred vertically.
        let xf = fit((160, 90), (100, 100));
        // Canvas pixels map back into the picture: centre to centre, the
        // left edge to the picture's, the top bar outside it.
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(near(xf.apply(50.0, 50.0), (80.0, 45.0)));
        assert!(near(xf.apply(0.0, 50.0), (0.0, 45.0)));
        assert!(xf.apply(50.0, 10.0).1 < 0.0);
    }
}
