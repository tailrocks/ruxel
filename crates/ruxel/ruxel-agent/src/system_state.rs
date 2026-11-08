use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct SystemState {
    packages: Option<HashMap<String, String>>,
    units: Option<HashMap<String, UnitState>>,
    candidates: HashMap<String, String>,
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

    pub(crate) fn package_version(&mut self, name: &str) -> Option<String> {
        self.ensure_packages().ok()?;
        self.packages.as_ref()?.get(name).cloned()
    }

    pub(crate) fn missing_packages(
        &mut self,
        names: &[String],
        latest: bool,
    ) -> Result<Vec<String>, String> {
        let requests: Vec<_> = names
            .iter()
            .map(|name| match name.split_once('=') {
                Some((package, version)) if !package.is_empty() && !version.is_empty() => {
                    Ok((name, package, Some(version)))
                }
                Some(_) => Err(format!("apt: invalid package version request {name:?}")),
                None => Ok((name, name.as_str(), None)),
            })
            .collect::<Result<_, _>>()?;
        self.ensure_packages()?;
        if latest {
            let unpinned: Vec<_> = requests
                .iter()
                .filter(|(_, _, version)| version.is_none())
                .map(|(name, _, _)| (*name).clone())
                .collect();
            self.ensure_candidates(&unpinned)?;
        }
        let packages = self.packages.as_ref().ok_or("packages not initialized")?;
        Ok(requests
            .iter()
            .filter(|(_, package, version)| match packages.get(*package) {
                None => true,
                Some(installed) if version.is_some() => version != &Some(installed.as_str()),
                Some(installed) if latest => self
                    .candidates
                    .get(*package)
                    .is_some_and(|candidate| candidate != installed && candidate != "(none)"),
                Some(_) => false,
            })
            .map(|(name, _, _)| (*name).clone())
            .collect())
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

    pub(crate) fn invalidate_packages(&mut self) {
        self.packages = None;
        self.candidates.clear();
    }

    pub(crate) fn invalidate_units(&mut self) {
        self.units = None;
    }

    fn ensure_packages(&mut self) -> Result<(), String> {
        self.ensure_packages_with(|| {
            std::fs::read_to_string("/var/lib/dpkg/status")
                .map_err(|error| format!("read dpkg status: {error}"))
        })
    }

    fn ensure_packages_with(
        &mut self,
        load: impl FnOnce() -> Result<String, String>,
    ) -> Result<(), String> {
        if self.packages.is_some() {
            return Ok(());
        }
        let text = load()?;
        self.packages = Some(parse_dpkg_status(&text));
        #[cfg(test)]
        {
            self.package_builds += 1;
        }
        Ok(())
    }

    fn ensure_candidates(&mut self, names: &[String]) -> Result<(), String> {
        let missing: Vec<_> = names
            .iter()
            .filter(|name| !self.candidates.contains_key(*name))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let output = std::process::Command::new("apt-cache")
            .arg("policy")
            .args(missing)
            .output()
            .map_err(|error| format!("apt-cache policy: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "apt-cache policy failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        self.candidates
            .extend(parse_apt_policy(&String::from_utf8_lossy(&output.stdout)));
        Ok(())
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

fn parse_dpkg_status(text: &str) -> HashMap<String, String> {
    let mut packages = HashMap::new();
    for paragraph in text.split("\n\n") {
        let mut name = None;
        let mut version = None;
        let mut architecture = None;
        let mut installed = false;
        for line in paragraph.lines() {
            if let Some(value) = line.strip_prefix("Package: ") {
                name = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("Version: ") {
                version = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("Architecture: ") {
                architecture = Some(value);
            } else if line == "Status: install ok installed" {
                installed = true;
            }
        }
        if let (true, Some(name), Some(version)) = (installed, name, version) {
            if let Some(architecture) = architecture {
                packages.insert(format!("{name}:{architecture}"), version.clone());
            }
            packages.insert(name, version);
        }
    }
    packages
}

fn parse_apt_policy(text: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut package = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && line.ends_with(':') {
            package = Some(line.trim_end_matches(':').to_string());
        } else if let Some(candidate) = line.trim().strip_prefix("Candidate: ")
            && let Some(package) = &package
        {
            result.insert(package.clone(), candidate.to_string());
        }
    }
    result
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
