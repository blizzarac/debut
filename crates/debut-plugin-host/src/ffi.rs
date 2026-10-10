//! Safe wrappers over the C hosts in `csrc/`.

use debut_platform::plugin_host::PluginParamInfo;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};

type ParamCb = extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    *const c_char,
    c_int,
    c_double,
    c_double,
    c_double,
);

extern "C" {
    fn debut_protect_stdout() -> c_int;

    fn debut_ofx_count(path: *const c_char, err: *mut c_char, cap: c_int) -> c_int;
    fn debut_ofx_describe(
        path: *const c_char,
        index: c_int,
        id: *mut c_char,
        label: *mut c_char,
        cap: c_int,
        cb: ParamCb,
        ctx: *mut c_void,
        err: *mut c_char,
        errcap: c_int,
    ) -> *mut c_void;
    fn debut_ofx_instance(
        descriptor: *mut c_void,
        fps: c_double,
        err: *mut c_char,
        errcap: c_int,
    ) -> *mut c_void;
    fn debut_ofx_param(inst: *mut c_void, name: *const c_char, comp: c_int, v: c_double) -> c_int;
    fn debut_ofx_render(
        inst: *mut c_void,
        time: c_double,
        fps: c_double,
        w: c_int,
        h: c_int,
        rgba: *mut f32,
        err: *mut c_char,
        errcap: c_int,
    ) -> c_int;

    fn debut_ofx_destroy(inst: *mut c_void);

    fn debut_clap_count(path: *const c_char, err: *mut c_char, cap: c_int) -> c_int;
    fn debut_clap_open(
        path: *const c_char,
        index: c_int,
        id: *mut c_char,
        name: *mut c_char,
        cap: c_int,
        cb: ParamCb,
        ctx: *mut c_void,
        rate: c_double,
        max_frames: c_int,
        err: *mut c_char,
        errcap: c_int,
    ) -> *mut c_void;
    fn debut_clap_param(inst: *mut c_void, name: *const c_char, v: c_double) -> c_int;
    fn debut_clap_process(
        inst: *mut c_void,
        buf: *mut f32,
        channels: c_int,
        frames: c_int,
    ) -> c_int;
    fn debut_clap_close(inst: *mut c_void);
}

const CAP: usize = 512;

fn cstring(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("{s:?} contains a NUL byte"))
}

fn text(buf: &[c_char]) -> String {
    // SAFETY: the C side always NUL-terminates within the buffer (snprintf).
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

extern "C" fn collect(
    ctx: *mut c_void,
    name: *const c_char,
    label: *const c_char,
    kind: *const c_char,
    _dims: c_int,
    default: c_double,
    min: c_double,
    max: c_double,
) {
    // SAFETY: `ctx` is the `Vec` passed by `describe_*` below, alive for the
    // call; the strings are NUL-terminated and valid for the callback.
    let (params, s) = unsafe {
        (
            &mut *(ctx as *mut Vec<PluginParamInfo>),
            |p: *const c_char| CStr::from_ptr(p).to_string_lossy().into_owned(),
        )
    };
    params.push(PluginParamInfo {
        name: s(name),
        label: s(label),
        kind: s(kind),
        default,
        min,
        max,
    });
}

/// Move the process's stdout aside for the protocol and point fd 1 at stderr,
/// so a plugin's `printf` can't corrupt replies. Returns the protocol fd.
pub fn protect_stdout() -> Option<i32> {
    // SAFETY: plain dup/dup2 on the process's own descriptors.
    let fd = unsafe { debut_protect_stdout() };
    (fd >= 0).then_some(fd)
}

/// A described OpenFX plugin; lives as long as the process (the binary stays loaded).
#[derive(Clone, Copy)]
pub struct OfxDescriptor(*mut c_void);

pub struct OfxInstance(*mut c_void);

pub struct Described<D> {
    pub handle: D,
    pub id: String,
    pub name: String,
    pub params: Vec<PluginParamInfo>,
}

pub fn ofx_count(path: &str) -> Result<u32, String> {
    let p = cstring(path)?;
    let mut err = [0 as c_char; CAP];
    // SAFETY: valid C strings and buffer sizes.
    let n = unsafe { debut_ofx_count(p.as_ptr(), err.as_mut_ptr(), CAP as c_int) };
    if n < 0 {
        Err(text(&err))
    } else {
        Ok(n as u32)
    }
}

pub fn ofx_describe(path: &str, index: u32) -> Result<Described<OfxDescriptor>, String> {
    let p = cstring(path)?;
    let (mut id, mut name, mut err) = ([0 as c_char; CAP], [0 as c_char; CAP], [0 as c_char; CAP]);
    let mut params: Vec<PluginParamInfo> = Vec::new();
    // SAFETY: buffers are CAP long; `params` outlives the call.
    let d = unsafe {
        debut_ofx_describe(
            p.as_ptr(),
            index as c_int,
            id.as_mut_ptr(),
            name.as_mut_ptr(),
            CAP as c_int,
            collect,
            &mut params as *mut _ as *mut c_void,
            err.as_mut_ptr(),
            CAP as c_int,
        )
    };
    if d.is_null() {
        return Err(text(&err));
    }
    Ok(Described {
        handle: OfxDescriptor(d),
        id: text(&id),
        name: text(&name),
        params,
    })
}

impl OfxDescriptor {
    pub fn instance(self, fps: f64) -> Result<OfxInstance, String> {
        let mut err = [0 as c_char; CAP];
        // SAFETY: `self.0` came from debut_ofx_describe and is never freed.
        let i = unsafe { debut_ofx_instance(self.0, fps, err.as_mut_ptr(), CAP as c_int) };
        if i.is_null() {
            Err(text(&err))
        } else {
            Ok(OfxInstance(i))
        }
    }
}

impl OfxInstance {
    /// Set `name` or component `name[i]`; false when the plugin has no such parameter.
    pub fn set(&mut self, name: &str, v: f64) -> bool {
        let (base, comp) = split_component(name);
        let Ok(n) = cstring(base) else { return false };
        // SAFETY: live instance, valid C string.
        unsafe { debut_ofx_param(self.0, n.as_ptr(), comp, v) == 0 }
    }

    pub fn render(
        &mut self,
        frame: f64,
        fps: f64,
        w: u32,
        h: u32,
        rgba: &mut [f32],
    ) -> Result<(), String> {
        if rgba.len() != w as usize * h as usize * 4 {
            return Err("frame size does not match the pixel count".into());
        }
        let mut err = [0 as c_char; CAP];
        // SAFETY: `rgba` holds w*h*4 floats, checked above.
        let r = unsafe {
            debut_ofx_render(
                self.0,
                frame,
                fps,
                w as c_int,
                h as c_int,
                rgba.as_mut_ptr(),
                err.as_mut_ptr(),
                CAP as c_int,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err(text(&err))
        }
    }
}

impl Drop for OfxInstance {
    fn drop(&mut self) {
        // SAFETY: created by debut_ofx_instance, dropped once.
        unsafe { debut_ofx_destroy(self.0) }
    }
}

/// "colour[2]" -> ("colour", 2); "gain" -> ("gain", 0).
fn split_component(name: &str) -> (&str, c_int) {
    if let Some(open) = name.rfind('[') {
        if let Some(inner) = name[open + 1..].strip_suffix(']') {
            if let Ok(i) = inner.parse() {
                return (&name[..open], i);
            }
        }
    }
    (name, 0)
}

pub struct ClapInstance(*mut c_void);

pub fn clap_count(path: &str) -> Result<u32, String> {
    let p = cstring(path)?;
    let mut err = [0 as c_char; CAP];
    // SAFETY: valid C strings and buffer sizes.
    let n = unsafe { debut_clap_count(p.as_ptr(), err.as_mut_ptr(), CAP as c_int) };
    if n < 0 {
        Err(text(&err))
    } else {
        Ok(n as u32)
    }
}

/// Create plugin `index`; with `rate` > 0 it is activated for processing.
pub fn clap_open(
    path: &str,
    index: u32,
    rate: f64,
    max_frames: u32,
) -> Result<Described<ClapInstance>, String> {
    let p = cstring(path)?;
    let (mut id, mut name, mut err) = ([0 as c_char; CAP], [0 as c_char; CAP], [0 as c_char; CAP]);
    let mut params: Vec<PluginParamInfo> = Vec::new();
    // SAFETY: buffers are CAP long; `params` outlives the call.
    let i = unsafe {
        debut_clap_open(
            p.as_ptr(),
            index as c_int,
            id.as_mut_ptr(),
            name.as_mut_ptr(),
            CAP as c_int,
            collect,
            &mut params as *mut _ as *mut c_void,
            rate,
            max_frames as c_int,
            err.as_mut_ptr(),
            CAP as c_int,
        )
    };
    if i.is_null() {
        return Err(text(&err));
    }
    Ok(Described {
        handle: ClapInstance(i),
        id: text(&id),
        name: text(&name),
        params,
    })
}

impl ClapInstance {
    pub fn set(&mut self, name: &str, v: f64) -> bool {
        let Ok(n) = cstring(name) else { return false };
        // SAFETY: live instance, valid C string.
        unsafe { debut_clap_param(self.0, n.as_ptr(), v) == 0 }
    }

    pub fn process(&mut self, samples: &mut [f32], channels: u32) -> Result<(), String> {
        let frames = samples.len() / channels.max(1) as usize;
        // SAFETY: `samples` holds frames*channels floats.
        let r = unsafe {
            debut_clap_process(
                self.0,
                samples.as_mut_ptr(),
                channels as c_int,
                frames as c_int,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err("the plugin reported a processing error".into())
        }
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        // SAFETY: created by debut_clap_open, dropped once.
        unsafe { debut_clap_close(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_names_split() {
        assert_eq!(split_component("gain"), ("gain", 0));
        assert_eq!(split_component("colour[2]"), ("colour", 2));
        assert_eq!(split_component("odd[x]"), ("odd[x]", 0));
    }
}
