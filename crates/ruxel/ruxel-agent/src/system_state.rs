use std::collections::HashMap;

mod apt;
mod apt_policy;
#[cfg(test)]
use apt_policy::{parse_dpkg_status, policy_state};

type PolicyRecord = (String, Option<String>, Option<String>);

#[derive(Default)]
pub(crate) struct SystemState {
    packages: Option<HashMap<String, String>>,
    units: Option<HashMap<String, UnitState>>,
    candidates: HashMap<String, String>,
    selector_installed: HashMap<String, Option<String>>,
    selector_identity: HashMap<String, String>,
    glob_policy: HashMap<String, Vec<PolicyRecord>>,
    native_architecture: Option<String>,
    #[cfg(test)]
    package_builds: usize,
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
            packages: Some(HashMap::from([("pkg".into(), "1".into())])),
            candidates: HashMap::from([("pkg".into(), "1".into())]),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn test_packages_invalidated(&self) -> bool {
        self.packages.is_none() && self.candidates.is_empty()
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
        let units = systemctl(&[
            "list-units",
            "--all",
            "--type=service",
            "--no-legend",
            "--plain",
        ])?;
        let files = systemctl(&[
            "list-unit-files",
            "--type=service",
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
