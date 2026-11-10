use std::collections::HashMap;

/// Installed packages from a dpkg status database, keyed `name:arch`.
pub(super) fn parse_dpkg_status(text: &str) -> HashMap<String, String> {
    let mut packages = HashMap::new();
    for paragraph in text.split("\n\n") {
        let mut name = None;
        let mut version = None;
        let mut architecture = None;
        let mut installed = false;
        for line in paragraph.lines() {
            if let Some(value) = line.strip_prefix("Package: ") {
                name = Some(value);
            } else if let Some(value) = line.strip_prefix("Version: ") {
                version = Some(value);
            } else if let Some(value) = line.strip_prefix("Architecture: ") {
                architecture = Some(value);
            } else if let Some(value) = line.strip_prefix("Status: ") {
                // Only healthy installations: wanted (install or hold), no
                // error flag such as reinstreq, and fully installed.
                installed = matches!(
                    value.split_whitespace().collect::<Vec<_>>().as_slice(),
                    ["install" | "hold", "ok", "installed"]
                );
            }
        }
        if let (true, Some(name), Some(version), Some(architecture)) =
            (installed, name, version, architecture)
        {
            packages.insert(format!("{name}:{architecture}"), version.to_owned());
        }
    }
    packages
}

/// Candidate versions from one batched `apt-cache policy` call, keyed by the
/// stanza header (`name` for native and `all` packages, `name:arch`
/// otherwise).
pub(super) fn parse_policy(text: &str) -> Result<HashMap<String, String>, String> {
    let mut candidates = HashMap::new();
    let mut headers = std::collections::HashSet::new();
    let mut header: Option<&str> = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && !line.is_empty() {
            let name = line
                .strip_suffix(':')
                .ok_or_else(|| format!("apt: unrecognized policy line {line:?}"))?;
            if !headers.insert(name) {
                return Err(format!("apt: duplicate policy stanza for {name:?}"));
            }
            header = Some(name);
        } else if let Some(value) = line.trim().strip_prefix("Candidate: ") {
            let name = header.ok_or("apt: policy candidate outside a stanza")?;
            if candidates
                .insert(name.to_owned(), value.to_owned())
                .is_some()
            {
                return Err(format!("apt: duplicate policy candidate for {name:?}"));
            }
        }
    }
    Ok(candidates)
}
