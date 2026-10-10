//! Hardware video decoding through FFmpeg's hwaccel API (NFR-09): a device
//! context (VAAPI, NVDEC via CUDA, Vulkan, VideoToolbox, D3D11VA) attached to
//! the decoder before it opens. libavcodec then negotiates the device's
//! surface format, and if the driver turns the stream down it drops that
//! format and decodes in software by itself; frames that do come from the
//! device are copied to system memory for the scaler.

use ffmpeg_next::ffi;
use std::ffi::{CStr, CString};
use std::ptr;

/// Device APIs tried in order on this OS (`DEBUT_HWACCEL` overrides, as a
/// comma-separated list).
pub fn candidates() -> Vec<String> {
    if let Ok(list) = std::env::var("DEBUT_HWACCEL") {
        return list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    let names: &[&str] = if cfg!(target_os = "macos") {
        &["videotoolbox"]
    } else if cfg!(target_os = "windows") {
        &["d3d11va", "cuda", "dxva2"]
    } else {
        &["vaapi", "cuda", "vulkan"]
    };
    names.iter().map(|s| s.to_string()).collect()
}

/// An open hardware device context.
pub struct Device {
    buf: *mut ffi::AVBufferRef,
    kind: ffi::AVHWDeviceType,
    pub name: String,
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: `buf` came from av_hwdevice_ctx_create and is unreferenced once.
        unsafe { ffi::av_buffer_unref(&mut self.buf) }
    }
}

// The device context is reference-counted by FFmpeg and only used by the
// decoder that owns it.
unsafe impl Send for Device {}

/// Open a device of type `name` ("vaapi", "cuda", ...).
pub fn open_device(name: &str) -> Result<Device, String> {
    let c = CString::new(name).map_err(|_| "bad device name".to_string())?;
    // SAFETY: plain FFmpeg calls with valid arguments; `buf` is written on success.
    unsafe {
        let kind = ffi::av_hwdevice_find_type_by_name(c.as_ptr());
        if kind == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
            return Err(format!("FFmpeg has no {name} support"));
        }
        let mut buf = ptr::null_mut();
        let r = ffi::av_hwdevice_ctx_create(&mut buf, kind, ptr::null(), ptr::null_mut(), 0);
        if r < 0 || buf.is_null() {
            return Err(format!(
                "no {name} device ({})",
                ffmpeg_next::Error::from(r)
            ));
        }
        Ok(Device {
            buf,
            kind,
            name: name.to_string(),
        })
    }
}

/// The surface format `codec` decodes to on devices like `device`, if it can.
///
/// # Safety
/// `codec` must be a valid codec pointer.
pub unsafe fn surface_format(
    codec: *const ffi::AVCodec,
    device: &Device,
) -> Option<ffi::AVPixelFormat> {
    let mut i = 0;
    loop {
        let config = ffi::avcodec_get_hw_config(codec, i);
        if config.is_null() {
            return None;
        }
        let c = &*config;
        let by_device = c.methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0;
        if by_device && c.device_type == device.kind {
            return Some(c.pix_fmt);
        }
        i += 1;
    }
}

/// Give an unopened decoder context the device.
///
/// # Safety
/// `ctx` must be a valid, not yet opened codec context.
pub unsafe fn attach(ctx: *mut ffi::AVCodecContext, device: &Device) {
    (*ctx).hw_device_ctx = ffi::av_buffer_ref(device.buf);
}

/// Copy a device frame into a system-memory frame (keeping its timestamps).
///
/// # Safety
/// `src` must be a decoded frame on a hardware device.
pub unsafe fn download(
    src: *const ffi::AVFrame,
    dst: *mut ffi::AVFrame,
) -> Result<(), ffmpeg_next::Error> {
    let r = ffi::av_hwframe_transfer_data(dst, src, 0);
    if r < 0 {
        return Err(ffmpeg_next::Error::from(r));
    }
    ffi::av_frame_copy_props(dst, src);
    Ok(())
}

/// Whether a decoded frame lives on a hardware device.
///
/// # Safety
/// `frame` must be a valid frame.
pub unsafe fn on_device(frame: *const ffi::AVFrame) -> bool {
    !(*frame).hw_frames_ctx.is_null()
}

/// Human name of a device type (for reports).
pub fn type_name(device: &Device) -> String {
    // SAFETY: returns a static string or null.
    unsafe {
        let p = ffi::av_hwdevice_get_type_name(device.kind);
        if p.is_null() {
            device.name.clone()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Whether any candidate device opens on this machine (cached).
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| candidates().iter().any(|n| open_device(n).is_ok()))
}
