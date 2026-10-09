//! Third-party plugins, run out-of-process so a crash never takes the app down
//! (FX-15, AUD-09, NFR-07, NFR-12). Desktop: OpenFX, VST3, AU. Browser: sandboxed WASM only.

use debut_core::Result;

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    OpenFx,
    Vst3,
    AudioUnit,
    Wasm,
}

pub trait PluginHost: Send + Sync {
    fn scan(&self) -> Result<Vec<PluginInfo>>;
}
