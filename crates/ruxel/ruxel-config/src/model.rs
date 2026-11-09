//! Strict guard and SSH configuration shared with admission.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "SshInput", into = "SshInput")]
pub struct Ssh {
    user: String,
    identity_file: String,
    known_hosts_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SshInput {
    #[serde(default = "root_user")]
    user: String,
    identity_file: String,
    known_hosts_file: String,
}

fn root_user() -> String {
    "root".into()
}

impl TryFrom<SshInput> for Ssh {
    type Error = &'static str;

    fn try_from(value: SshInput) -> Result<Self, Self::Error> {
        if value.user.is_empty()
            || value.user.starts_with('-')
            || !value
                .user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("invalid SSH user");
        }
        if value.identity_file.is_empty()
            || value.known_hosts_file.is_empty()
            || value.identity_file.contains('\0')
            || value.known_hosts_file.contains('\0')
        {
            return Err("invalid SSH path");
        }
        Ok(Self {
            user: value.user,
            identity_file: value.identity_file,
            known_hosts_file: value.known_hosts_file,
        })
    }
}

impl From<Ssh> for SshInput {
    fn from(value: Ssh) -> Self {
        Self {
            user: value.user,
            identity_file: value.identity_file,
            known_hosts_file: value.known_hosts_file,
        }
    }
}

impl Ssh {
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }
    #[must_use]
    pub fn identity_file(&self) -> &str {
        &self.identity_file
    }
    #[must_use]
    pub fn known_hosts_file(&self) -> &str {
        &self.known_hosts_file
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "GuardInput", into = "GuardInput")]
pub struct Guard {
    allow: Vec<String>,
    deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardInput {
    allow: Vec<String>,
    deny: Vec<String>,
}

impl TryFrom<GuardInput> for Guard {
    type Error = ruxel_guard::PolicyError;
    fn try_from(value: GuardInput) -> Result<Self, Self::Error> {
        ruxel_guard::Policy::new(&value.allow, &value.deny)?;
        Ok(Self {
            allow: value.allow,
            deny: value.deny,
        })
    }
}

impl From<Guard> for GuardInput {
    fn from(value: Guard) -> Self {
        Self {
            allow: value.allow,
            deny: value.deny,
        }
    }
}

impl Guard {
    #[must_use]
    pub fn allow(&self) -> &[String] {
        &self.allow
    }
    #[must_use]
    pub fn deny(&self) -> &[String] {
        &self.deny
    }
}

#[cfg(test)]
mod tests {
    use super::{Guard, Ssh};
    use serde_json::json;

    #[test]
    fn ssh_is_strict_and_defaults_the_user() {
        let ssh: Ssh = serde_json::from_value(
            json!({"identityFile":"keys/id", "knownHostsFile":"keys/hosts"}),
        )
        .unwrap();
        assert_eq!(ssh.user(), "root");
        assert_eq!(ssh.identity_file(), "keys/id");
        assert_eq!(ssh.known_hosts_file(), "keys/hosts");
        for value in [
            json!({"identityFile":"", "knownHostsFile":"x"}),
            json!({"identityFile":"x", "knownHostsFile":"x", "user":"-bad"}),
            json!({"identityFile":"x", "knownHostsFile":"x", "extra":true}),
            json!({"identityFile":"x"}),
        ] {
            assert!(serde_json::from_value::<Ssh>(value).is_err());
        }
    }

    #[test]
    fn guard_rejects_invalid_networks_and_unknown_fields() {
        let guard: Guard =
            serde_json::from_value(json!({"allow":["192.0.2.0/24"],"deny":["198.51.100.1"]}))
                .unwrap();
        assert_eq!(guard.allow(), ["192.0.2.0/24"]);
        assert_eq!(guard.deny(), ["198.51.100.1"]);
        for value in [
            json!({"allow":["invalid"],"deny":[]}),
            json!({"allow":[],"deny":[],"extra":true}),
            json!({"allow":[]}),
        ] {
            assert!(serde_json::from_value::<Guard>(value).is_err());
        }
    }
}
