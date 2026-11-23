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
    /// SSH trust files and their ancestors must be root/effective-user-owned,
    /// without symlinks or group/other write access. Linux supports ext2/3/4,
    /// XFS, Btrfs, tmpfs and ramfs; macOS requires local APFS/HFS and refuses
    /// ACL allow entries. Other filesystems fail closed. Root and the effective
    /// user must remain trusted throughout the returned targets' lifetime.
    /// Missing `known_hosts` files are created with mode 0600 under checked
    /// parents so OpenSSH accept-new cannot inherit unsafe write permissions.
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
        let mut refusals: Vec<_> = hosts
            .iter()
            .filter_map(|spec| {
                self.policy.decide(spec.address).err().map(|rule| Refusal {
                    host: spec.name.clone(),
                    resolved: spec.address.to_string(),
                    rule,
                })
            })
            .collect();
        refusals.extend(scan::deny_hits(&self.policy, literals).into_iter().map(
            |(literal, rule)| Refusal {
                host: "literal".into(),
                resolved: literal,
                rule,
            },
        ));
        if !refusals.is_empty() {
            return Err(Refusals(refusals));
        }
        let mut targets = Vec::new();
        let mux_dir = crate::mux::prepare(&self.mux_dir).map_err(|rule| {
            Refusals(vec![Refusal {
                host: String::new(),
                resolved: String::new(),
                rule,
            }])
        })?;
        for spec in hosts {
            let result = (|| {
                resolve::validate_paths(spec)?;
                crate::trusted_path::identity(&spec.identity_file)?;
                crate::trusted_path::known_hosts(&spec.known_hosts_file)?;
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
                resolve::validate_path(&path)?;
                if path.as_os_str().len() > 100 {
                    return Err("mux socket path exceeds 100 bytes".into());
                }
                let args = resolve::build_args(spec, &path, ArgsMode::Normal);
                resolve::check_ssh_g(&run(&args)?, spec)?;
                Ok(path)
            })();
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
        let root = crate::mux::test_directory().unwrap();
        let dir = root.join("mux");
        std::fs::write(root.join("identity"), b"key").unwrap();
        let policy = Policy::new(&["192.0.2.0/24".into()], &["198.51.100.0/24".into()]).unwrap();
        let guard = Guard::new(policy, dir.clone());
        let spec = |address: &str| HostSpec {
            name: address.into(),
            address: address.parse().unwrap(),
            port: 22,
            user: "test".into(),
            identity_file: root.join("identity"),
            known_hosts_file: root.join("known"),
        };
        let calls = Cell::new(0);
        let simulate = |args: &[OsString]| {
            calls.set(calls.get() + 1);
            assert_eq!(args.last().unwrap(), "192.0.2.10");
            Ok(format!(
                "hostname 192.0.2.10\nuser test\nport 22\nforwardagent no\nstricthostkeychecking true\nuserknownhostsfile {}\n",
                root.join("known").display()
            ))
        };
        let result = guard.admit_with(&[spec("192.0.2.10"), spec("198.51.100.10")], &[], simulate);
        assert!(result.is_err());
        assert_eq!(calls.get(), 0);
        assert!(!dir.exists());
        assert!(!root.join("known").exists());
        let result = guard.admit_with(&[spec("192.0.2.10")], &[], simulate);
        assert_eq!(result.unwrap().len(), 1);
        std::fs::remove_dir(dir).unwrap();
        std::fs::remove_file(root.join("known")).unwrap();
        std::fs::remove_file(root.join("identity")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn all_policy_refusals_precede_admission_effects() {
        let root = crate::mux::test_directory().unwrap();
        let dir = root.join("mux");
        let guard = Guard::new(
            Policy::new(&["192.0.2.0/24".into()], &["198.51.100.0/24".into()]).unwrap(),
            dir.clone(),
        );
        let spec = |address: &str| HostSpec {
            name: address.into(),
            address: address.parse().unwrap(),
            port: 22,
            user: "test".into(),
            identity_file: root.join("missing-identity"),
            known_hosts_file: root.join("known"),
        };
        let calls = Cell::new(0);
        let result = guard.admit_with(
            &[
                spec("192.0.2.10"),
                spec("198.51.100.10"),
                spec("203.0.113.10"),
            ],
            &["198.51.100.20"],
            |_| {
                calls.set(calls.get() + 1);
                Err("unexpected".into())
            },
        );
        let refusals = result.unwrap_err().0;
        assert_eq!(refusals.len(), 3);
        assert_eq!(refusals[0].resolved, "198.51.100.10");
        assert_eq!(refusals[1].resolved, "203.0.113.10");
        assert_eq!(refusals[2].resolved, "198.51.100.20");
        assert_eq!(calls.get(), 0);
        assert!(!dir.exists());
        assert!(!root.join("known").exists());
        let result = guard.admit_with(&[spec("192.0.2.10")], &["198.51.100.20"], |_| {
            calls.set(calls.get() + 1);
            Err("unexpected".into())
        });
        assert_eq!(result.unwrap_err().0[0].host, "literal");
        assert_eq!(calls.get(), 0);
        assert!(!dir.exists());
        assert!(!root.join("known").exists());
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn untrusted_known_hosts_are_refused_before_ssh() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::mux::test_directory().unwrap();
        let known = root.join("known");
        std::fs::write(root.join("identity"), b"key").unwrap();
        std::fs::write(&known, b"untrusted keys").unwrap();
        std::fs::set_permissions(&known, std::fs::Permissions::from_mode(0o666)).unwrap();
        let guard = Guard::new(
            Policy::new(&["192.0.2.0/24".into()], &[]).unwrap(),
            root.join("mux"),
        );
        let spec = HostSpec {
            name: "test".into(),
            address: "192.0.2.10".parse().unwrap(),
            port: 22,
            user: "test".into(),
            identity_file: root.join("identity"),
            known_hosts_file: known,
        };
        let calls = Cell::new(0);
        let result = guard.admit_with(&[spec], &[], |_| {
            calls.set(calls.get() + 1);
            Err("unexpected SSH invocation".into())
        });
        assert!(result.unwrap_err().0[0].rule.contains("write access"));
        assert_eq!(calls.get(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod directory_tests {
    #![allow(clippy::unwrap_used, reason = "fixture setup must succeed")]
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn rejects_existing_directory_with_group_access() {
        let root = crate::mux::test_directory().unwrap();
        let path = root.join("mux");
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
        let guard = Guard::new(Policy::new(&[], &[]).unwrap(), path.clone());
        let refusal = guard.admit_all(&[], &[]).unwrap_err();
        assert!(refusal.0[0].rule.contains("mode 0700"));
        std::fs::remove_dir(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

#[cfg(test)]
mod long_path_test {
    #![allow(clippy::unwrap_used, reason = "fixture setup must succeed")]
    use super::*;
    use std::cell::Cell;

    #[test]
    fn rejects_long_socket_path_before_ssh() {
        let root = crate::mux::test_directory().unwrap();
        let dir = root.join("a".repeat(100));
        std::fs::write(root.join("identity"), b"key").unwrap();
        let guard = Guard::new(
            Policy::new(&["192.0.2.0/24".into()], &[]).unwrap(),
            dir.clone(),
        );
        let host = HostSpec {
            name: "local-test".into(),
            address: "192.0.2.10".parse().unwrap(),
            port: 22,
            user: "test".into(),
            identity_file: root.join("identity"),
            known_hosts_file: root.join("known"),
        };
        let calls = Cell::new(0);
        let result = guard.admit_with(&[host], &[], |_| {
            calls.set(calls.get() + 1);
            Err("unexpected".into())
        });
        let refusal = result.unwrap_err();
        assert!(refusal.0[0].rule.contains("mux socket path exceeds"));
        assert_eq!(calls.get(), 0);
        std::fs::remove_dir(dir).unwrap();
        std::fs::remove_file(root.join("known")).unwrap();
        std::fs::remove_file(root.join("identity")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
