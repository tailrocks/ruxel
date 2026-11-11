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
