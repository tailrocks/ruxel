use super::SystemState;
use super::apt_policy::{PolicyEntry, parse_dpkg_status, parse_policy};
use super::apt_request::PackageRequest;
use super::apt_version;

/// What `apt-get install` must do to satisfy a request list.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct AptPlan {
    /// Requests that are not satisfied, as written.
    pub missing: Vec<String>,
    /// Arguments for one transaction: every missing request as written, plus
    /// every satisfied request pinned to its installed version so that APT
    /// can neither upgrade nor remove it as a side effect.
    pub transaction: Vec<String>,
}

impl SystemState {
    /// Installed version of `name` or `name:arch`.
    pub(crate) fn package_version(&mut self, selector: &str) -> Option<String> {
        let request = PackageRequest::parse(selector)
            .ok()
            .filter(|request| request.version().is_none())?;
        self.ensure_packages().ok()?;
        self.installed(&request).map(str::to_owned)
    }

    /// # Errors
    /// Returns snapshot or `apt-cache policy` failures.
    ///
    /// `environment` is the task environment that apt-get runs with (for
    /// example `APT_CONFIG`); candidate lookups use the same one.
    pub(crate) fn plan_packages(
        &mut self,
        requests: &[PackageRequest],
        latest: bool,
        environment: &[(String, String)],
    ) -> Result<AptPlan, String> {
        self.ensure_packages()?;
        if self.candidate_environment != environment {
            self.candidates.clear();
            self.candidate_environment = environment.to_vec();
        }
        // Policy is needed to compare installed unpinned requests with
        // `latest`, and to refuse missing requests that apt could satisfy only
        // through another package (virtual) or not at all.
        let unknown: Vec<_> = requests
            .iter()
            .filter(|request| {
                let installed = self.installed(request).is_some();
                (!installed || (latest && request.version().is_none()))
                    && !self.candidates.contains_key(&self.policy_header(request))
            })
            .map(PackageRequest::selector)
            .collect();
        self.query_candidates(&unknown, environment)?;
        let mut plan = AptPlan::default();
        for request in requests {
            let installed = self.installed(request);
            let policy = self.candidates.get(&self.policy_header(request));
            let satisfied = match (installed, request.version()) {
                (None, _) => {
                    Self::require_available(request, policy)?;
                    false
                }
                (Some(installed), Some(version)) => apt_version::equal(installed, version),
                (Some(installed), None) if latest => {
                    let policy = policy.ok_or_else(|| {
                        format!("apt: no policy candidate for {:?}", request.selector())
                    })?;
                    policy
                        .candidate
                        .as_deref()
                        .is_none_or(|candidate| apt_version::equal(candidate, installed))
                }
                (Some(_), None) => true,
            };
            if satisfied {
                let installed = installed.ok_or("apt: satisfied request is not installed")?;
                plan.transaction.push(request.pinned(installed));
            } else {
                plan.missing.push(request.to_string());
                plan.transaction.push(request.to_string());
            }
        }
        Ok(plan)
    }

    /// A missing request must name a package with its own installable
    /// version: the requested one, or a default candidate when unpinned.
    /// Virtual packages have neither.
    fn require_available(
        request: &PackageRequest,
        policy: Option<&PolicyEntry>,
    ) -> Result<(), String> {
        let available = policy.is_some_and(|policy| match request.version() {
            Some(version) => policy
                .versions
                .iter()
                .any(|candidate| apt_version::equal(candidate, version)),
            None => policy.candidate.is_some(),
        });
        if available {
            Ok(())
        } else {
            Err(format!(
                "apt: {request} is not available; virtual and unavailable packages or versions are not supported, request the providing package"
            ))
        }
    }

    fn native_arch(&self) -> &str {
        self.native_architecture.as_deref().unwrap_or_default()
    }

    /// The dpkg status entry a request names: its explicit architecture, or
    /// for an unqualified name the native architecture, then `all`.
    fn installed(&self, request: &PackageRequest) -> Option<&str> {
        let packages = self.packages.as_ref()?;
        let entry = |arch: &str| packages.get(&format!("{}:{arch}", request.name()));
        match request.arch() {
            Some(arch) => entry(arch),
            None => entry(self.native_arch()).or_else(|| entry("all")),
        }
        .map(String::as_str)
    }

    fn policy_header(&self, request: &PackageRequest) -> String {
        match request.arch() {
            Some(arch) if arch != self.native_arch() => request.selector(),
            _ => request.name().to_owned(),
        }
    }

    pub(crate) fn invalidate_packages(&mut self) {
        self.packages = None;
        self.candidates.clear();
    }

    /// The cached apt candidate policy. A write under the apt preference
    /// or source paths changes which version apt would choose without
    /// changing what is installed, so exactly the policy cache is
    /// dropped and the next plan re-reads it.
    pub(crate) fn invalidate_candidates(&mut self) {
        self.candidates.clear();
    }

    /// The host's native architecture, known once the package snapshot is
    /// loaded.
    pub(crate) fn native_architecture(&self) -> Option<&str> {
        self.native_architecture.as_deref()
    }

    fn ensure_packages(&mut self) -> Result<(), String> {
        if self.packages.is_some() {
            return Ok(());
        }
        #[cfg(test)]
        if let Some(status) = self.dpkg_status.clone() {
            self.native_architecture = Some("amd64".into());
            return self.ensure_packages_with(|| {
                status
                    .lock()
                    .map(|status| status.clone())
                    .map_err(|error| error.to_string())
            });
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
        })
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

    /// One batched `apt-cache policy` call for every selector.
    fn query_candidates(
        &mut self,
        selectors: &[String],
        environment: &[(String, String)],
    ) -> Result<(), String> {
        if selectors.is_empty() {
            return Ok(());
        }
        #[cfg(test)]
        let program = self.apt_cache.clone().unwrap_or_else(|| "apt-cache".into());
        #[cfg(not(test))]
        let program = "apt-cache";
        let output = std::process::Command::new(program)
            .arg("policy")
            .args(selectors)
            .envs(environment.iter().cloned())
            .env("LC_ALL", "C")
            .output()
            .map_err(|error| format!("apt-cache policy: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "apt-cache policy failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        self.candidates
            .extend(parse_policy(&String::from_utf8_lossy(&output.stdout))?);
        Ok(())
    }
}
