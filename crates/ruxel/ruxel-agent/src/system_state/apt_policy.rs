use std::collections::HashMap;

pub(super) fn parse_dpkg_status(text: &str) -> HashMap<String, String> {
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

pub(super) fn glob_policy_stanza(
    text: &str,
) -> Result<(String, Option<String>, Option<String>), String> {
    let name = text
        .lines()
        .next()
        .and_then(|line| line.strip_suffix(':'))
        .ok_or("apt: invalid glob policy output")?;
    policy_state(name, text)
}

pub(super) fn policy_state(
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
