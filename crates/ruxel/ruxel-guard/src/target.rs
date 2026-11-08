use crate::{
    Policy,
    resolve::{self, ArgsMode},
    scan,
};
use std::{
    ffi::OsString,
    net::IpAddr,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct HostSpec {
    pub name: String,
    pub address: IpAddr,
    pub port: u16,
    pub user: String,
    pub identity_file: PathBuf,
    pub known_hosts_file: PathBuf,
}
#[derive(Debug)]
pub struct GuardedTarget {
    spec: HostSpec,
    control_path: PathBuf,
}
impl GuardedTarget {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.spec.name
    }
    #[must_use]
    pub fn address(&self) -> IpAddr {
        self.spec.address
    }
    #[must_use]
    pub fn known_hosts_file(&self) -> &Path {
        &self.spec.known_hosts_file
    }
    #[must_use]
    pub fn ssh_args(&self) -> Vec<OsString> {
        resolve::build_args(&self.spec, &self.control_path, ArgsMode::Normal)
    }
    #[must_use]
    pub fn ssh_args_fresh(&self) -> Vec<OsString> {
        resolve::build_args(&self.spec, &self.control_path, ArgsMode::Fresh)
    }
    #[must_use]
    pub fn trust_args(&self) -> Vec<OsString> {
        resolve::build_args(&self.spec, &self.control_path, ArgsMode::Trust)
    }
}
#[derive(Debug)]
pub struct Refusal {
    pub host: String,
    pub resolved: String,
    pub rule: String,
}
#[derive(Debug)]
pub struct Refusals(pub Vec<Refusal>);
#[derive(Debug)]
pub struct Guard {
    policy: Policy,
    mux_dir: PathBuf,
}
impl Guard {
    #[must_use]
    pub fn new(policy: Policy, mux_dir: PathBuf) -> Self {
        Self { policy, mux_dir }
    }
    /// Admit every host and reject the entire batch on any refusal.
    ///
    /// # Errors
    /// Returns all detected refusals if any host or literal is unsafe.
    pub fn admit_all(
        &self,
        hosts: &[HostSpec],
        literals: &[&str],
    ) -> Result<Vec<GuardedTarget>, Refusals> {
        self.admit_with(hosts, literals, resolve::run_ssh_g)
    }

    fn admit_with(
        &self,
        hosts: &[HostSpec],
        literals: &[&str],
        mut run: impl FnMut(&[OsString]) -> Result<String, String>,
    ) -> Result<Vec<GuardedTarget>, Refusals> {
        let mut refusals = Vec::new();
        let mut targets = Vec::new();
        let mux_dir = crate::mux::prepare(&self.mux_dir).map_err(|rule| {
            Refusals(vec![Refusal {
                host: String::new(),
                resolved: String::new(),
                rule,
            }])
        })?;
        for spec in hosts {
            let result = self.policy.decide(spec.address).and_then(|()| {
                let mut bytes = [0u8; 6];
                getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
                let suffix: String = bytes
                    .iter()
                    .flat_map(|b| {
                        [
                            char::from(b"0123456789abcdef"[usize::from(b >> 4)]),
                            char::from(b"0123456789abcdef"[usize::from(b & 15)]),
                        ]
                    })
                    .collect();
                let path = mux_dir.join(&suffix);
                if path.as_os_str().len() > 100 {
                    return Err("mux socket path exceeds 100 bytes".into());
                }
                let args = resolve::build_args(spec, &path, ArgsMode::Normal);
                resolve::check_ssh_g(&run(&args)?, spec)?;
                Ok(path)
            });
            match result {
                Ok(control_path) => targets.push(GuardedTarget {
                    spec: HostSpec {
                        name: spec.name.clone(),
                        address: spec.address,
                        port: spec.port,
                        user: spec.user.clone(),
                        identity_file: spec.identity_file.clone(),
                        known_hosts_file: spec.known_hosts_file.clone(),
                    },
                    control_path,
                }),
                Err(rule) => refusals.push(Refusal {
                    host: spec.name.clone(),
                    resolved: spec.address.to_string(),
                    rule,
                }),
            }
        }
        refusals.extend(scan::deny_hits(&self.policy, literals).into_iter().map(
            |(literal, rule)| Refusal {
                host: "literal".into(),
                resolved: literal,
                rule,
            },
        ));
        if refusals.is_empty() {
            Ok(targets)
        } else {
            Err(Refusals(refusals))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "fixture construction must succeed")]
    use super::*;
    use std::cell::Cell;

    #[test]
    fn admission_uses_simulated_ssh_only_for_allowed_hosts() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("g{}-{suffix:x}", std::process::id()));
        let policy = Policy::new(&["192.0.2.0/24".into()], &["198.51.100.0/24".into()]).unwrap();
        let guard = Guard::new(policy, dir.clone());
        let spec = |address: &str| HostSpec {
            name: address.into(),
            address: address.parse().unwrap(),
            port: 22,
            user: "test".into(),
            identity_file: "/tmp/key".into(),
            known_hosts_file: "/tmp/known".into(),
        };
        let calls = Cell::new(0);
        let simulate = |args: &[OsString]| {
            calls.set(calls.get() + 1);
            assert_eq!(args.last().unwrap(), "192.0.2.10");
            Ok("hostname 192.0.2.10\nuser test\nport 22\nforwardagent no\nstricthostkeychecking true\nuserknownhostsfile /tmp/known\n".into())
        };
        let result = guard.admit_with(&[spec("192.0.2.10"), spec("198.51.100.10")], &[], simulate);
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        let result = guard.admit_with(&[spec("192.0.2.10")], &[], simulate);
        assert_eq!(result.unwrap().len(), 1);
        std::fs::remove_dir(dir).unwrap();
    }
}

#[cfg(test)]
mod directory_tests {
    #![allow(clippy::unwrap_used, reason = "fixture setup must succeed")]
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn rejects_existing_directory_with_group_access() {
        let path = std::env::temp_dir().join(format!("ruxel-guard-mode-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
        let guard = Guard::new(Policy::new(&[], &[]).unwrap(), path.clone());
        let refusal = guard.admit_all(&[], &[]).unwrap_err();
        assert!(refusal.0[0].rule.contains("mode 0700"));
        std::fs::remove_dir(path).unwrap();
    }
}

#[cfg(test)]
mod long_path_test {
    #![allow(clippy::unwrap_used, reason = "fixture setup must succeed")]
    use super::*;
    use std::cell::Cell;

    #[test]
    fn rejects_long_socket_path_before_ssh() {
        let root = std::env::temp_dir().join(format!("guardlong{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let dir = root.join("a".repeat(100));
        let guard = Guard::new(
            Policy::new(&["192.0.2.0/24".into()], &[]).unwrap(),
            dir.clone(),
        );
        let host = HostSpec {
            name: "local-test".into(),
            address: "192.0.2.10".parse().unwrap(),
            port: 22,
            user: "test".into(),
            identity_file: "/tmp/key".into(),
            known_hosts_file: "/tmp/known".into(),
        };
        let calls = Cell::new(0);
        let result = guard.admit_with(&[host], &[], |_| {
            calls.set(calls.get() + 1);
            Err("unexpected".into())
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 0);
        std::fs::remove_dir(dir).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
