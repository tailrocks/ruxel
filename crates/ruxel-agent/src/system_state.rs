//! Lazy per-run system snapshots shared by module checks and ledger probes.

use std::collections::HashMap;

#[derive(Default)]
pub struct SystemState {
    packages: Option<HashMap<String, String>>,
    units: Option<HashMap<String, UnitState>>,
    candidates: HashMap<String, String>,
    #[cfg(test)]
    package_builds: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnitState {
    pub active: bool,
    pub enabled: bool,
}

impl SystemState {
    pub fn package_version(&mut self, name: &str) -> Option<String> {
        self.ensure_packages().ok()?;
        self.packages.as_ref()?.get(name).cloned()
    }

    pub fn missing_packages(
        &mut self,
        names: &[String],
        latest: bool,
    ) -> Result<Vec<String>, String> {
        self.ensure_packages()?;
        if latest {
            self.ensure_candidates(names)?;
        }
        let packages = self.packages.as_ref().expect("packages initialized");
        Ok(names
            .iter()
            .filter(|name| match packages.get(*name) {
                None => true,
                Some(installed) if latest => self
                    .candidates
                    .get(*name)
                    .is_some_and(|candidate| candidate != installed && candidate != "(none)"),
                Some(_) => false,
            })
            .cloned()
            .collect())
    }

    pub fn unit(&mut self, name: &str) -> Result<UnitState, String> {
        self.ensure_units()?;
        Ok(self
            .units
            .as_ref()
            .expect("units initialized")
            .get(name)
            .copied()
            .unwrap_or_default())
    }

    pub fn invalidate_packages(&mut self) {
        self.packages = None;
        self.candidates.clear();
    }

    pub fn invalidate_units(&mut self) {
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
    text.split("\n\n")
        .filter_map(|paragraph| {
            let mut name = None;
            let mut version = None;
            let mut installed = false;
            for line in paragraph.lines() {
                if let Some(value) = line.strip_prefix("Package: ") {
                    name = Some(value.to_string());
                } else if let Some(value) = line.strip_prefix("Version: ") {
                    version = Some(value.to_string());
                } else if line == "Status: install ok installed" {
                    installed = true;
                }
            }
            installed.then(|| Some((name?, version?))).flatten()
        })
        .collect()
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
mod tests {
    use super::*;

    #[test]
    fn parses_many_packages_once() {
        let source = "Package: one\nStatus: install ok installed\nVersion: 1.0\n\nPackage: old\nStatus: deinstall ok config-files\nVersion: 0.1\n\nPackage: two\nStatus: install ok installed\nVersion: 2.0\n";
        let mut state = SystemState::default();
        state.ensure_packages_with(|| Ok(source.into())).unwrap();
        state
            .ensure_packages_with(|| panic!("snapshot must not rebuild"))
            .unwrap();
        assert_eq!(state.package_builds, 1);
        let parsed = state.packages.unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["one"], "1.0");
        assert_eq!(parsed["two"], "2.0");
    }

    #[test]
    fn parses_batched_policy_and_systemd_snapshots() {
        let policy = parse_apt_policy(
            "one:\n  Installed: 1\n  Candidate: 2\ntwo:\n  Installed: 3\n  Candidate: 3\n",
        );
        assert_eq!(policy["one"], "2");
        let units = parse_systemd(
            "a.service loaded active running A\nb.service loaded inactive dead B\n",
            "a.service enabled enabled\nb.service disabled enabled\n",
        );
        assert_eq!(
            units["a.service"],
            UnitState {
                active: true,
                enabled: true
            }
        );
        assert_eq!(
            units["b.service"],
            UnitState {
                active: false,
                enabled: false
            }
        );
    }

    #[test]
    fn single_threaded_snapshot_lookup_scale() {
        let source = (0..10_000)
            .map(|index| {
                format!(
                    "Package: package-{index}\nStatus: install ok installed\nVersion: 1.{index}\n\n"
                )
            })
            .collect::<String>();
        let started = std::time::Instant::now();
        let packages = parse_dpkg_status(&source);
        for index in 0..10_000 {
            assert!(packages.contains_key(&format!("package-{index}")));
        }
        let elapsed = started.elapsed();
        eprintln!("10k package snapshot + 10k lookups: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_secs(5));
    }
}
