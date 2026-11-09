use super::SystemState;
use super::apt_policy::{glob_policy_stanza, parse_dpkg_status, policy_state, simulate_glob};
use std::collections::HashMap;

impl SystemState {
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
        for (_, package, _) in &requests {
            if package.contains(['*', '?', '[']) {
                self.ensure_glob_policy(package)?;
            }
        }
        let qualified: Vec<_> = requests
            .iter()
            .filter(|(_, package, _)| package.contains(':') && !package.contains(['*', '?', '[']))
            .map(|(_, package, _)| (*package).to_string())
            .collect();
        self.ensure_candidates(&qualified)?;
        self.validate_package_requests(&requests, latest)?;
        if latest {
            let unpinned: Vec<_> = requests
                .iter()
                .filter(|(_, _, version)| version.is_none())
                .filter(|(_, package, _)| !package.contains(['*', '?', '[']))
                .map(|(name, _, _)| (*name).clone())
                .collect();
            self.ensure_candidates(&unpinned)?;
        }
        let packages = self.packages.as_ref().ok_or("packages not initialized")?;
        let mut missing = Vec::new();
        for (name, package, version) in requests {
            if package.contains(['*', '?', '[']) {
                if simulate_glob(name, latest, version.is_some())? {
                    missing.push(name.clone());
                }
                continue;
            }
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
                let selected: Vec<&str> = if let Some(records) = self.glob_policy.get(*package) {
                    records
                        .iter()
                        .map(|(identity, _, _)| identity.as_str())
                        .collect()
                } else {
                    vec![
                        self.selector_identity
                            .get(*package)
                            .map_or(*package, String::as_str),
                    ]
                };
                for selected in selected {
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
                    let entry = normalized_pins.entry(normalized).or_insert((None, false));
                    if (version.is_some() && entry.0.is_some() && entry.0 != *version)
                        || (latest
                            && ((version.is_some() && entry.1)
                                || (version.is_none() && entry.0.is_some())))
                    {
                        return Err(format!(
                            "apt: conflicting versions requested for {normalized:?}"
                        ));
                    }
                    if version.is_some() {
                        entry.0 = *version;
                    } else {
                        entry.1 = true;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn invalidate_packages(&mut self) {
        self.packages = None;
        self.candidates.clear();
        self.selector_installed.clear();
        self.selector_identity.clear();
        self.glob_policy.clear();
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

    pub(super) fn ensure_packages_with(
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

    fn ensure_glob_policy(&mut self, name: &str) -> Result<(), String> {
        if self.glob_policy.contains_key(name) {
            return Ok(());
        }
        let output = std::process::Command::new("apt-cache")
            .args(["policy", name])
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
        let mut records = Vec::new();
        let mut stanza = String::new();
        for line in text.lines() {
            if !line.starts_with(char::is_whitespace) && line.ends_with(':') && !stanza.is_empty() {
                records.push(glob_policy_stanza(&stanza)?);
                stanza.clear();
            }
            stanza.push_str(line);
            stanza.push('\n');
        }
        if !stanza.is_empty() {
            records.push(glob_policy_stanza(&stanza)?);
        }
        self.glob_policy.insert(name.to_string(), records);
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
}
