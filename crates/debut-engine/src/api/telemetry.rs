//! Telemetry settings and the report (NFR-15): see `crate::telemetry`.

use super::*;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TelemetryDto {
    pub enabled: bool,
    pub endpoint: Option<String>,
    /// Exactly what Send would post (JSON), so the user can read it first.
    pub report: Option<String>,
}

impl Session {
    pub fn telemetry_status(&self) -> TelemetryDto {
        TelemetryDto {
            enabled: self.telemetry.is_some(),
            endpoint: self.settings.telemetry_endpoint.clone(),
            report: self.telemetry.as_ref().map(|r| r.json()),
        }
    }

    /// Opt in or out. Opting out also erases what was collected.
    pub fn set_telemetry(&mut self, on: bool) -> Result<TelemetryDto, String> {
        self.settings.telemetry = on;
        self.settings.save(self.platform.as_ref())?;
        if on {
            if self.telemetry.is_none() {
                let mut r = crate::telemetry::Report::load(self.platform.as_ref());
                r.sessions += 1;
                r.save(self.platform.as_ref())?;
                self.telemetry = Some(r);
            }
        } else {
            self.telemetry = None;
            crate::telemetry::Report::new().save(self.platform.as_ref())?;
        }
        Ok(self.telemetry_status())
    }

    pub fn set_telemetry_endpoint(&mut self, endpoint: Option<String>) -> Result<(), String> {
        self.settings.telemetry_endpoint = endpoint.filter(|e| !e.trim().is_empty());
        self.settings.save(self.platform.as_ref())
    }

    /// Post the report to the configured endpoint (only when asked).
    pub fn send_telemetry(&mut self) -> Result<u16, String> {
        let report = self.telemetry.as_ref().ok_or("telemetry is off")?;
        let endpoint = self
            .settings
            .telemetry_endpoint
            .clone()
            .ok_or("no telemetry endpoint is set")?;
        crate::telemetry::send(self.platform.as_ref(), report, &endpoint)
    }

    /// Count a feature use, when the user opted in.
    pub fn note_feature(&mut self, name: &str) {
        if let Some(t) = self.telemetry.as_mut() {
            t.feature(name);
        }
    }

    /// Write the report to disk at most once a minute (from `tick`), and
    /// pick up crashes the panic hook recorded meanwhile.
    pub(crate) fn flush_telemetry(&mut self, force: bool) {
        let now = self.platform.now();
        if self.telemetry.is_none()
            || (!force
                && now.saturating_sub(self.telemetry_saved) < std::time::Duration::from_secs(60))
        {
            return;
        }
        self.telemetry_saved = now;
        let on_disk = crate::telemetry::Report::load(self.platform.as_ref());
        if let Some(t) = self.telemetry.as_mut() {
            for c in on_disk.crashes {
                if !t.crashes.contains(&c) {
                    t.crashes.push(c);
                }
            }
            let _ = t.save(self.platform.as_ref());
        }
    }
}
