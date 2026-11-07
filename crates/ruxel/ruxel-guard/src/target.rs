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
    pub fn name(&self) -> &str {
        &self.spec.name
    }
    pub fn address(&self) -> IpAddr {
        self.spec.address
    }
    pub fn known_hosts_file(&self) -> &Path {
        &self.spec.known_hosts_file
    }
    pub fn ssh_args(&self) -> Vec<OsString> {
        resolve::build_args(&self.spec, &self.control_path, ArgsMode::Normal)
    }
    pub fn ssh_args_fresh(&self) -> Vec<OsString> {
        resolve::build_args(&self.spec, &self.control_path, ArgsMode::Fresh)
    }
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
    pub fn new(policy: Policy, mux_dir: PathBuf) -> Self {
        Self { policy, mux_dir }
    }
    pub fn admit_all(
        &self,
        hosts: &[HostSpec],
        literals: &[&str],
    ) -> Result<Vec<GuardedTarget>, Refusals> {
        use std::os::unix::fs::DirBuilderExt;
        let mut refusals = Vec::new();
        let mut targets = Vec::new();
        if let Err(e) = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.mux_dir)
        {
            return Err(Refusals(vec![Refusal {
                host: String::new(),
                resolved: String::new(),
                rule: e.to_string(),
            }]));
        }
        for spec in hosts {
            let result = self.policy.decide(spec.address).and_then(|()| {
                let mut bytes = [0u8; 6];
                getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
                let suffix = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
                let mut path = self.mux_dir.join(&suffix);
                if path.as_os_str().len() > 100 {
                    path = std::env::temp_dir().join(format!("ruxel-mux-{suffix}"));
                }
                let args = resolve::build_args(spec, &path, ArgsMode::Normal);
                resolve::check_ssh_g(&resolve::run_ssh_g(&args)?, spec)?;
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
