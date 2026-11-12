use std::collections::HashMap;

mod apt;
mod apt_policy;
mod apt_request;
mod apt_version;
#[cfg(test)]
use apt_policy::{parse_dpkg_status, parse_policy};
pub(crate) use apt_request::PackageRequest;

#[derive(Default)]
pub(crate) struct SystemState {
    packages: Option<HashMap<String, String>>,
    units: Option<HashMap<String, UnitState>>,
    /// Run state a check-mode task predicted but did not perform. A later
    /// task of the same run judges against the state the run will leave
    /// behind: the reload handler a start's change queues runs after the
    /// start, so check must report the reload the start makes possible.
    predicted_activity: HashMap<String, bool>,
    /// The predicted enablement beside the predicted run state: the same
    /// enabled service written twice is two tasks of one run, and the
    /// second must judge against the enablement the first leaves behind
    /// exactly as apply performs it once.
    predicted_enabled: HashMap<String, bool>,
    candidates: HashMap<String, apt_policy::PolicyEntry>,
    /// Task environment the cached candidates were observed with.
    candidate_environment: Vec<(String, String)>,
    native_architecture: Option<String>,
    #[cfg(test)]
    package_builds: usize,
    /// Synthetic amd64 dpkg status database that replaces the host's.
    /// Replacement `apt-cache` executable.
    #[cfg(test)]
    apt_cache: Option<std::path::PathBuf>,
    #[cfg(test)]
    dpkg_status: Option<std::sync::Arc<std::sync::Mutex<String>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct UnitState {
    pub active: bool,
    pub enabled: bool,
}

impl SystemState {
    #[cfg(test)]
    pub(crate) fn with_test_packages() -> Self {
        Self {
            packages: Some(HashMap::from([("pkg:amd64".into(), "1".into())])),
            candidates: HashMap::from([(
                "pkg".into(),
                apt_policy::PolicyEntry {
                    candidate: Some("1".into()),
                    versions: vec!["1".into()],
                },
            )]),
            native_architecture: Some("amd64".into()),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_dpkg_status(status: std::sync::Arc<std::sync::Mutex<String>>) -> Self {
        Self {
            dpkg_status: Some(status),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn test_packages_invalidated(&self) -> bool {
        self.packages.is_none() && self.candidates.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn with_test_units() -> Self {
        Self {
            units: Some(HashMap::from([(
                "synthetic.service".into(),
                UnitState {
                    active: true,
                    enabled: true,
                },
            )])),
            ..Self::with_test_packages()
        }
    }

    #[cfg(test)]
    pub(crate) fn test_units_invalidated(&self) -> bool {
        self.units.is_none()
    }

    pub(crate) fn unit(&mut self, name: &str) -> Result<UnitState, String> {
        self.ensure_units()?;
        let units = self.units.as_ref().ok_or("units not initialized")?;
        Ok(units
            .get(name)
            .or_else(|| {
                (!name.contains('.'))
                    .then(|| format!("{name}.service"))
                    .as_ref()
                    .and_then(|qualified| units.get(qualified))
            })
            .copied()
            .unwrap_or_default())
    }

    pub(crate) fn invalidate_units(&mut self) {
        self.units = None;
    }

    /// The unit state a task judges against: the run state an earlier
    /// check-mode task predicted, when there is one, and the observed
    /// state otherwise.
    pub(crate) fn predicted_unit(&mut self, name: &str) -> Result<UnitState, String> {
        let mut unit = self.unit(name)?;
        if let Some(active) = self.predicted_activity.get(&unit_key(name)) {
            unit.active = *active;
        }
        if let Some(enabled) = self.predicted_enabled.get(&unit_key(name)) {
            unit.enabled = *enabled;
        }
        Ok(unit)
    }

    /// Records the run state a predicted change leaves behind, so the
    /// tasks after it judge against the state the run will leave.
    pub(crate) fn predict_activity(&mut self, name: &str, active: bool) {
        self.predicted_activity.insert(unit_key(name), active);
    }

    /// Records the enablement a predicted change leaves behind, beside
    /// the run state.
    pub(crate) fn predict_enablement(&mut self, name: &str, enabled: bool) {
        self.predicted_enabled.insert(unit_key(name), enabled);
    }

    /// A performed change makes its prediction stale: the observed state
    /// is the truth again.
    pub(crate) fn clear_prediction(&mut self, name: &str) {
        self.predicted_activity.remove(&unit_key(name));
        self.predicted_enabled.remove(&unit_key(name));
    }

    /// An opaque change made every prediction stale at once: a command or
    /// shell task can start, stop, enable and disable any unit behind the
    /// agent's back, so the overlays are dropped beside the snapshot.
    pub(crate) fn clear_predictions(&mut self) {
        self.predicted_activity.clear();
        self.predicted_enabled.clear();
    }

    fn ensure_units(&mut self) -> Result<(), String> {
        if self.units.is_some() {
            return Ok(());
        }
        // Timer units are listed too: a timer is enabled and disabled like
        // a service, and its enablement is only known if the unit is read.
        let units = systemctl(&[
            "list-units",
            "--all",
            "--type=service,timer",
            "--no-legend",
            "--plain",
        ])?;
        let files = systemctl(&[
            "list-unit-files",
            "--type=service,timer",
            "--no-legend",
            "--plain",
        ])?;
        self.units = Some(parse_systemd(&units, &files));
        Ok(())
    }
}

/// The key a prediction is carried under: a bare name names its service,
/// as `unit` resolves it against the probed map.
fn unit_key(name: &str) -> String {
    if name.contains('.') {
        name.to_owned()
    } else {
        format!("{name}.service")
    }
}

fn systemctl(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|error| format!("exec systemctl: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "systemctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_systemd(units: &str, files: &str) -> HashMap<String, UnitState> {
    let mut result = HashMap::new();
    for line in units.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() >= 4 {
            result
                .entry(columns[0].to_string())
                .or_insert(UnitState::default())
                .active = columns[2] == "active";
        }
    }
    for line in files.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() >= 2 {
            result
                .entry(columns[0].to_string())
                .or_insert(UnitState::default())
                .enabled = columns[1] == "enabled";
        }
    }
    result
}

#[cfg(test)]
mod tests;
