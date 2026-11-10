//! Admission of a remote destination against the compiled deny list.

use super::RemoteHost;
use anyhow::{Context, Result, anyhow, bail};
use std::process::Stdio;

/// Options that pin every post-admission invocation to the admitted address.
/// `HostName` expands % tokens, so `%%` keeps a scope suffix literal; proxies
/// and an inherited multiplexing master could reach another endpoint.
pub(super) fn pinned_options(address: &str) -> [String; 10] {
    [
        "-o".into(),
        format!("HostName={}", address.replace('%', "%%")),
        "-o".into(),
        "ProxyJump=none".into(),
        "-o".into(),
        "ProxyCommand=none".into(),
        "-o".into(),
        "ControlMaster=no".into(),
        "-o".into(),
        "ControlPath=none".into(),
    ]
}

impl RemoteHost {
    /// Resolve the destination with `ssh -G`, which reads configuration and
    /// makes no network contact, and refuse it unless it names a literal IP
    /// outside every deny entry with no proxy hop.
    pub(super) async fn admit(&mut self, policy: &ruxel_guard::Policy) -> Result<()> {
        let mut command = self.options();
        command
            .args(["-G", "--", &self.destination])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        // ConnectTimeout does not cover configuration evaluation (Match exec).
        let output = self.probe(command).await?;
        if !output.status.success() {
            bail!(
                "refused {}: ssh -G could not resolve the destination",
                self.destination
            );
        }
        // Unrelated values such as IdentityFile paths may be non-UTF-8; the
        // fields read below are validated strictly.
        let config = String::from_utf8_lossy(&output.stdout);
        self.admitted = Some(admit_resolved(&config, &self.destination, policy)?);
        Ok(())
    }
    /// Run the probe in its own process group and, on timeout, kill the whole
    /// group so `Match exec` helpers do not outlive it.
    async fn probe(&self, mut command: tokio::process::Command) -> Result<std::process::Output> {
        let child = command.spawn().context("ssh -G")?;
        let group = child
            .id()
            .and_then(|id| rustix::process::Pid::from_raw(id.cast_signed()));
        let Ok(output) =
            tokio::time::timeout(self.admission_timeout, child.wait_with_output()).await
        else {
            if let Some(group) = group {
                // The group may already be gone; the probe is refused either way.
                rustix::process::kill_process_group(group, rustix::process::Signal::KILL).ok();
            }
            bail!("refused {}: ssh -G timed out", self.destination)
        };
        output.context("ssh -G")
    }
}

/// Returns the admitted hostname, including any IPv6 scope suffix.
pub(super) fn admit_resolved(
    config: &str,
    destination: &str,
    policy: &ruxel_guard::Policy,
) -> Result<String> {
    let value = |key: &str| {
        config.lines().find_map(|line| {
            line.split_once(' ')
                .filter(|(name, _)| *name == key)
                .map(|(_, value)| value.trim())
        })
    };
    let hostname = value("hostname")
        .with_context(|| format!("refused {destination}: ssh -G reported no hostname"))?;
    let address = match hostname.split_once('%') {
        Some((address, scope)) if !scope.is_empty() => address
            .parse::<std::net::Ipv6Addr>()
            .ok()
            .map(std::net::IpAddr::V6),
        Some(_) => None,
        None => hostname.parse::<std::net::IpAddr>().ok(),
    }
    .ok_or_else(|| {
        anyhow!("refused {destination}: hostname {hostname:?} is not a literal IP address")
    })?;
    // The kernel connects 0.0.0.0 and :: to a local address that the deny
    // list would not see.
    if address.to_canonical().is_unspecified() {
        bail!("refused {destination}: hostname {hostname:?} is an unspecified address");
    }
    for key in ["proxyjump", "proxycommand"] {
        if value(key).is_some_and(|value| value != "none") {
            bail!("refused {destination}: {key} is set");
        }
    }
    policy
        .check_deny(address)
        .map_err(|rule| anyhow!("refused {destination} ({address}): {rule}"))?;
    Ok(hostname.to_owned())
}
