//! Third-party plugins (FX-15, AUD-09): scanning, OpenFX clip effects and
//! CLAP track inserts, all run by the platform's out-of-process host.

use super::*;
use debut_core::Curve;
use debut_platform::plugin_host::{PluginInfo, PluginKind, ScanResult};
use debut_project::{AudioPluginParam, PluginFx, PluginParam};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PluginDto {
    /// "openfx" or "clap".
    pub kind: String,
    pub path: String,
    pub index: u32,
    pub id: String,
    pub name: String,
    pub params: Vec<PluginParamDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PluginParamDto {
    pub name: String,
    pub label: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub animated: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PluginsDto {
    /// Whether this target hosts plugins at all.
    pub available: bool,
    pub plugins: Vec<PluginDto>,
    /// Binaries that could not be used: (path, why).
    pub problems: Vec<(String, String)>,
    /// Binaries the user approved to run (NFR-13).
    pub approved: Vec<String>,
}

fn kind_str(k: PluginKind) -> &'static str {
    match k {
        PluginKind::OpenFx => "openfx",
        PluginKind::Clap => "clap",
    }
}

/// Display ranges for a slider: what the plugin reports, narrowed when it
/// reports "anything" (an unbounded double).
fn slider_range(min: f64, max: f64, default: f64) -> (f64, f64) {
    let lo = if min.is_finite() && min > -1e6 {
        min
    } else {
        (default * 2.0).min(0.0)
    };
    let hi = if max.is_finite() && max < 1e6 {
        max
    } else {
        (default * 2.0).max(1.0)
    };
    (lo, hi.max(lo))
}

fn plugin_dto(p: &PluginInfo) -> PluginDto {
    PluginDto {
        kind: kind_str(p.plugin.kind).into(),
        path: p.plugin.path.clone(),
        index: p.plugin.index,
        id: p.id.clone(),
        name: p.name.clone(),
        params: p
            .params
            .iter()
            .map(|q| {
                let (min, max) = slider_range(q.min, q.max, q.default);
                PluginParamDto {
                    name: q.name.clone(),
                    label: q.label.clone(),
                    min,
                    max,
                    value: q.default,
                    animated: false,
                }
            })
            .collect(),
    }
}

/// A clip's plugin effect as the inspector shows it, evaluated at clip-local `t`.
pub(crate) fn plugin_fx_dto(p: &PluginFx, t: Rational) -> PluginDto {
    PluginDto {
        kind: "openfx".into(),
        path: p.path.clone(),
        index: p.index,
        id: p.id.clone(),
        name: p.name.clone(),
        params: p
            .params
            .iter()
            .map(|q| PluginParamDto {
                name: q.name.clone(),
                label: q.label.clone(),
                min: q.min,
                max: q.max,
                value: q.value.eval(t),
                animated: q.value.keys().len() > 1,
            })
            .collect(),
    }
}

/// A track insert's parameters (empty for the built-in inserts).
pub(crate) fn insert_params(e: &AudioEffect) -> Vec<PluginParamDto> {
    match e {
        AudioEffect::Plugin { params, .. } => params
            .iter()
            .map(|q| PluginParamDto {
                name: q.name.clone(),
                label: q.name.clone(),
                min: q.min,
                max: q.max,
                value: q.value,
                animated: false,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl Session {
    /// Look for plugins on the search path (slow: every binary is loaded, in
    /// the helper). The result is kept for `plugins`.
    pub fn scan_plugins(&mut self) -> Result<PluginsDto, String> {
        let Some(host) = self.platform.plugins() else {
            self.plugin_scan = Some(ScanResult::default());
            return Ok(PluginsDto::default());
        };
        let scan = host.scan().map_err(|e| e.to_string())?;
        self.plugin_scan = Some(scan);
        Ok(self.plugins())
    }

    /// The plugin host behind the approval check (NFR-13).
    pub(crate) fn plugin_host(&self) -> Option<Arc<dyn debut_platform::PluginHost>> {
        self.plugin_host
            .as_ref()
            .map(|h| Arc::clone(h) as Arc<dyn debut_platform::PluginHost>)
    }

    /// Approve the plugin binary at `path` as it is now (NFR-13): pinned by
    /// its SHA-256, so a changed file needs approving again. Saved in the
    /// user's settings.
    pub fn approve_plugin(&mut self, path: &str) -> Result<(), String> {
        let hash = crate::trust::sha256(self.store.as_ref(), path).map_err(|e| e.to_string())?;
        self.set_trusted(|t| {
            t.insert(path.to_string(), hash);
        })
    }

    /// Withdraw approval: the binary no longer runs.
    pub fn revoke_plugin(&mut self, path: &str) -> Result<(), String> {
        self.set_trusted(|t| {
            t.remove(path);
        })
    }

    fn set_trusted(
        &mut self,
        f: impl FnOnce(&mut std::collections::BTreeMap<String, String>),
    ) -> Result<(), String> {
        {
            let mut t = self.trusted.write().unwrap_or_else(|e| e.into_inner());
            f(&mut t);
            self.settings.trusted_plugins = t.clone();
        }
        if let Some(h) = &self.plugin_host {
            h.rehash();
        }
        self.settings.save(self.platform.as_ref())
    }

    /// Approved binaries and their pinned SHA-256.
    pub fn plugin_approvals(&self) -> Vec<(String, String)> {
        self.trusted
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect()
    }

    /// The last scan's result (empty before the first).
    pub fn plugins(&self) -> PluginsDto {
        let scan = self.plugin_scan.clone().unwrap_or_default();
        PluginsDto {
            available: self.platform.plugins().is_some(),
            approved: self
                .plugin_approvals()
                .into_iter()
                .map(|(p, _)| p)
                .collect(),
            plugins: scan.plugins.iter().map(plugin_dto).collect(),
            problems: scan
                .problems
                .iter()
                .map(|p| (p.path.clone(), p.error.clone()))
                .collect(),
        }
    }

    fn scanned(&self, kind: PluginKind, path: &str, index: u32) -> Result<PluginInfo, String> {
        self.plugin_scan
            .as_ref()
            .and_then(|s| {
                s.plugins.iter().find(|p| {
                    p.plugin.kind == kind && p.plugin.path == path && p.plugin.index == index
                })
            })
            .cloned()
            .ok_or_else(|| format!("no scanned plugin {path} #{index}; scan for plugins first"))
    }

    /// Add an OpenFX filter to a clip, with its default parameter values.
    pub fn add_plugin_effect(
        &mut self,
        track: &str,
        clip: &str,
        path: &str,
        index: u32,
    ) -> Result<(), String> {
        let info = self.scanned(PluginKind::OpenFx, path, index)?;
        let (target, clip_id, _) = self.clip_ref(track, clip)?;
        // Picking it from the scan list is the approval.
        self.approve_plugin(path)?;
        let effect = Effect::Plugin(PluginFx {
            path: path.into(),
            index,
            id: info.id.clone(),
            name: info.name.clone(),
            params: plugin_dto(&info)
                .params
                .into_iter()
                .map(|p| PluginParam {
                    name: p.name,
                    label: p.label,
                    min: p.min,
                    max: p.max,
                    value: Curve::constant(p.value),
                })
                .collect(),
        });
        self.exec(Command::AddEffect {
            target,
            clip: clip_id,
            effect,
            index: None,
        })
    }

    /// Set a plugin effect's parameter as a constant, or keyframe it at the playhead.
    pub fn set_plugin_param(
        &mut self,
        track: &str,
        clip: &str,
        effect: usize,
        name: &str,
        value: f64,
        keyframe: bool,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err("the value must be a number".into());
        }
        let fr = self.first_sequence()?.frame_rate;
        let playhead = self.playhead();
        let (target, clip_id, c) = self.clip_ref(track, clip)?;
        let at = keyframe.then(|| fr.snap(playhead) - c.timeline_in);
        let Some(Effect::Plugin(mut p)) = c.effects.get(effect).cloned() else {
            return Err("that effect is not a plugin".into());
        };
        if !p.set(name, at, value) {
            return Err(format!("{} has no parameter {name}", p.name));
        }
        self.exec(Command::ReplaceEffect {
            target,
            clip: clip_id,
            index: effect,
            effect: Effect::Plugin(p),
        })
    }

    /// Add a CLAP effect to a track's insert chain.
    pub fn add_plugin_insert(&mut self, track: &str, path: &str, index: u32) -> Result<(), String> {
        let info = self.scanned(PluginKind::Clap, path, index)?;
        let (target, mut effects) = self.track_inserts(track)?;
        self.approve_plugin(path)?;
        effects.push(AudioEffect::Plugin {
            path: path.into(),
            index,
            id: info.id.clone(),
            name: info.name.clone(),
            params: plugin_dto(&info)
                .params
                .into_iter()
                .map(|p| AudioPluginParam {
                    name: p.name,
                    min: p.min,
                    max: p.max,
                    value: p.value,
                })
                .collect(),
        });
        self.exec(Command::SetTrackAudio { target, effects })
    }

    /// Set a parameter of a track's plugin insert.
    pub fn set_insert_param(
        &mut self,
        track: &str,
        insert: usize,
        name: &str,
        value: f64,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err("the value must be a number".into());
        }
        let (target, mut effects) = self.track_inserts(track)?;
        let Some(AudioEffect::Plugin {
            params,
            name: plugin,
            ..
        }) = effects.get_mut(insert)
        else {
            return Err("that insert is not a plugin".into());
        };
        let Some(p) = params.iter_mut().find(|p| p.name == name) else {
            return Err(format!("{plugin} has no parameter {name}"));
        };
        p.value = value;
        self.exec(Command::SetTrackAudio { target, effects })
    }

    /// The last error a plugin effect reported while rendering, if any.
    pub fn plugin_error(&self) -> Option<String> {
        self.player.as_ref().and_then(|p| p.frames.plugin_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_ranges_get_a_usable_slider() {
        assert_eq!(slider_range(0.0, 4.0, 1.0), (0.0, 4.0));
        assert_eq!(slider_range(f64::MIN, f64::MAX, 1.0), (0.0, 2.0));
        assert_eq!(slider_range(-1e300, 1e300, 10.0), (0.0, 20.0));
        assert_eq!(slider_range(f64::MIN, 5.0, -3.0), (-6.0, 5.0));
    }
}
