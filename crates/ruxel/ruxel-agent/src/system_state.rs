use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct SystemState {
    packages: Option<HashMap<String, String>>,
    units: Option<HashMap<String, UnitState>>,
    candidates: HashMap<String, String>,
    selector_installed: HashMap<String, Option<String>>,
    selector_identity: HashMap<String, String>,
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
            if let Some(version) = version
                && version.contains(['*', '?', '['])
            {
                glob::Pattern::new(version).map_err(|error| {
                    format!("apt: invalid version pattern for {package:?}: {error}")
                })?;
            }
        }
        self.ensure_packages()?;
        let qualified: Vec<_> = requests
            .iter()
            .filter(|(_, package, _)| package.contains(':'))
            .map(|(_, package, _)| (*package).to_string())
            .collect();
        self.ensure_candidates(&qualified)?;
        self.validate_package_requests(&requests, latest)?;
        if latest {
            let unpinned: Vec<_> = requests
                .iter()
                .filter(|(_, _, version)| version.is_none())
                .map(|(name, _, _)| (*name).clone())
                .collect();
            self.ensure_candidates(&unpinned)?;
        }
        let packages = self.packages.as_ref().ok_or("packages not initialized")?;
        let mut missing = Vec::new();
        for (name, package, version) in requests {
            let installed = self.installed_for_selector(packages, package)?;
            let needs_install = match installed {
                None => true,
                Some(installed) if version.is_some() => {
                    let pattern = version.unwrap_or_default();
                    if pattern.contains(['*', '?', '[']) {
                        !glob::Pattern::new(pattern).is_ok_and(|p| p.matches(installed))
                    } else {
                        pattern != installed
                    }
                }
                Some(installed) if latest => {
                    let candidate = self
                        .candidates
                        .get(package)
                        .ok_or_else(|| format!("apt: no policy candidate for {package:?}"))?;
                    candidate != installed && candidate != "(none)"
                }
                Some(_) => false,
            };
            if needs_install {
                missing.push(name.clone());
            }
        }
        Ok(missing)
    }

    fn installed_for_selector<'a>(
        &'a self,
        packages: &'a HashMap<String, String>,
        package: &str,
    ) -> Result<Option<&'a String>, String> {
        if !package.contains(':') {
            return Ok(packages.get(package));
        }
        let policy = self
            .selector_installed
            .get(package)
            .ok_or_else(|| format!("apt: no policy state for {package:?}"))?
            .as_ref();
        let identity = self
            .selector_identity
            .get(package)
            .ok_or_else(|| format!("apt: no policy identity for {package:?}"))?;
        let installed = packages.get(identity);
        if let (Some(policy), Some(actual)) = (policy, installed)
            && policy != actual
        {
            return Err(format!("apt: inconsistent installed state for {package:?}"));
        }
        Ok(installed.filter(|_| policy.is_some()))
    }

    fn validate_package_requests(
        &self,
        requests: &[(&String, &str, Option<&str>)],
        latest: bool,
    ) -> Result<(), String> {
        let mut pins = HashMap::new();
        for (_, package, version) in requests {
            if let Some(version) = version
                && let Some(previous) = pins.insert(*package, *version)
                && previous != *version
            {
                return Err(format!(
                    "apt: conflicting versions requested for {package:?}"
                ));
            }
        }
        if let Some(architecture) = &self.native_architecture {
            let mut normalized_pins = HashMap::new();
            for (_, package, version) in requests {
                let selected = self
                    .selector_identity
                    .get(*package)
                    .map_or(*package, String::as_str);
                let normalized = selected
                    .rsplit_once(':')
                    .filter(|(name, requested)| {
                        *requested == "native"
                            || requested == architecture
                            || (*requested == "all"
                                && self.packages.as_ref().is_some_and(|packages| {
                                    packages.contains_key(&format!("{name}:all"))
                                }))
                    })
                    .map_or(selected, |(name, _)| name);
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
        Ok(())
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
        self.selector_installed.clear();
        self.selector_identity.clear();
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
        for name in names {
            if self.candidates.contains_key(name) {
                continue;
            }
            let output = std::process::Command::new("apt-cache")
                .arg("policy")
                .arg(name)
                .env("LC_ALL", "C")
                .output()
                .map_err(|error| format!("apt-cache policy: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "apt-cache policy failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            let text = String::from_utf8_lossy(&output.stdout);
            let (identity, installed, candidate) = policy_state(name, &text)?;
            if let Some(candidate) = candidate {
                self.candidates.insert(name.clone(), candidate);
            }
            if name.contains(':') {
                self.selector_identity.insert(name.clone(), identity);
                self.selector_installed.insert(name.clone(), installed);
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

fn policy_state(
    name: &str,
    text: &str,
) -> Result<(String, Option<String>, Option<String>), String> {
    let mut identity = None;
    let mut installed = None;
    let mut candidate = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && line.ends_with(':') {
            if identity.replace(line.trim_end_matches(':')).is_some() {
                return Err(format!("apt: ambiguous policy output for {name:?}"));
            }
        } else if let Some(value) = line.trim().strip_prefix("Installed: ") {
            if installed.replace(value).is_some() {
                return Err(format!("apt: ambiguous policy output for {name:?}"));
            }
        } else if let Some(value) = line.trim().strip_prefix("Candidate: ")
            && candidate.replace(value).is_some()
        {
            return Err(format!("apt: ambiguous policy output for {name:?}"));
        }
    }
    let Some(identity) = identity else {
        if text.trim().is_empty() {
            return Ok((name.to_string(), None, None));
        }
        return Err(format!("apt: invalid policy output for {name:?}"));
    };
    let base = name.split_once(':').map_or(name, |(base, _)| base);
    if identity != name
        && identity != base
        && !(name.ends_with(":any") && identity.starts_with(&format!("{base}:")))
    {
        return Err(format!("apt: unexpected policy identity for {name:?}"));
    }
    let installed =
        installed.ok_or_else(|| format!("apt: missing installed policy state for {name:?}"))?;
    let candidate =
        candidate.ok_or_else(|| format!("apt: missing candidate policy state for {name:?}"))?;
    Ok((
        identity.to_string(),
        (installed != "(none)").then(|| installed.to_string()),
        Some(candidate.to_string()),
    ))
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
