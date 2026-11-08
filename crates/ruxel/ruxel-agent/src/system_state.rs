use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct SystemState {
    packages: Option<HashMap<String, String>>,
    units: Option<HashMap<String, UnitState>>,
    candidates: HashMap<String, String>,
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
        for (_, package, version) in &requests {
            if package.ends_with(":any") {
                return Err(format!(
                    "apt: :any package selectors are not supported: {package:?}"
                ));
            }
            if version.is_some_and(|version| version.contains(['?', '[', ']'])) {
                return Err(format!("apt: unsupported version pattern for {package:?}"));
            }
        }
        let mut pins = HashMap::new();
        for (_, package, version) in &requests {
            if let Some(version) = version
                && let Some(previous) = pins.insert(*package, *version)
                && previous != *version
            {
                return Err(format!(
                    "apt: conflicting versions requested for {package:?}"
                ));
            }
        }
        self.ensure_packages()?;
        if let Some(architecture) = &self.native_architecture {
            let mut normalized_pins = HashMap::new();
            for (_, package, version) in &requests {
                let normalized = package
                    .rsplit_once(':')
                    .filter(|(name, requested)| {
                        *requested == "native"
                            || requested == architecture
                            || (*requested == "all"
                                && self.packages.as_ref().is_some_and(|packages| {
                                    packages.contains_key(&format!("{name}:all"))
                                }))
                    })
                    .map_or(*package, |(name, _)| name);
                if let Some(previous) = normalized_pins.insert(normalized, *version)
                    && ((previous.is_some() && version.is_some() && previous != *version)
                        || (latest && previous.is_some() != version.is_some()))
                {
                    return Err(format!(
                        "apt: conflicting versions requested for {normalized:?}"
                    ));
                }
            }
        }
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
            .filter(|(_, package, version)| {
                match packages.get(*package).or_else(|| {
                    package
                        .strip_suffix(":native")
                        .and_then(|name| packages.get(name))
                }) {
                    None => true,
                    Some(installed) if version.is_some() => {
                        let pattern = version.unwrap_or_default();
                        if pattern.contains('*') {
                            let regex = format!(
                                "^{}$",
                                pattern
                                    .split('*')
                                    .map(regex_lite::escape)
                                    .collect::<Vec<_>>()
                                    .join(".*")
                            );
                            !regex_lite::Regex::new(&regex)
                                .is_ok_and(|regex| regex.is_match(installed))
                        } else {
                            pattern != installed
                        }
                    }
                    Some(installed) if latest => self
                        .candidates
                        .get(*package)
                        .is_some_and(|candidate| candidate != installed && candidate != "(none)"),
                    Some(_) => false,
                }
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
        if self.packages.is_some() {
            return Ok(());
        }
        let output = std::process::Command::new("dpkg")
            .arg("--print-architecture")
            .output()
            .map_err(|error| format!("dpkg --print-architecture: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "dpkg --print-architecture failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let architecture = String::from_utf8(output.stdout)
            .map_err(|error| format!("invalid dpkg architecture: {error}"))?;
        let architecture = architecture.trim();
        if architecture.is_empty() {
            return Err("dpkg --print-architecture returned an empty architecture".to_string());
        }
        self.native_architecture = Some(architecture.to_string());
        self.ensure_packages_with(|| {
            std::fs::read_to_string("/var/lib/dpkg/status")
                .map_err(|error| format!("read dpkg status: {error}"))
        })?;
        let packages = self.packages.as_mut().ok_or("packages not initialized")?;
        let names: Vec<_> = packages
            .keys()
            .filter_map(|key| key.split_once(':').map(|(name, _)| name.to_string()))
            .collect();
        for name in names {
            let qualified = format!("{name}:{architecture}");
            let version = packages
                .get(&qualified)
                .or_else(|| packages.get(&format!("{name}:all")))
                .cloned();
            if let Some(version) = version {
                packages.insert(qualified, version.clone());
                packages.insert(name, version);
            } else {
                packages.remove(&name);
            }
        }
        Ok(())
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
            .args(&missing)
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
        if let Some(architecture) = &self.native_architecture {
            for name in missing {
                if let Some((package, requested_architecture)) = name.rsplit_once(':')
                    && (requested_architecture == architecture
                        || requested_architecture == "native"
                        || requested_architecture == "all")
                    && let Some(candidate) = self.candidates.get(package).cloned()
                {
                    self.candidates.entry(name.clone()).or_insert(candidate);
                }
            }
        }
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
