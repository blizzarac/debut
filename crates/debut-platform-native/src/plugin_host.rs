//! OpenFX / VST3 / AU host, out-of-process (FX-15, AUD-09, NFR-07). Pending.

use debut_core::Result;
use debut_platform::plugin_host::{PluginHost, PluginInfo};

pub struct NativePluginHost;

impl PluginHost for NativePluginHost {
    fn scan(&self) -> Result<Vec<PluginInfo>> {
        Ok(Vec::new())
    }
}
