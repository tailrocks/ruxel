use super::apt_request::PackageRequest;
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

/// The packages `apt-mark showhold` lists: one package selector per line.
///
/// A hold the host reports is a package the agent must be able to name, so
/// a line that is not a package selector is refused instead of accepted as
/// a hold.
///
/// # Errors
/// Returns an error naming the first line that is not a package selector.
pub(crate) fn parse_showhold(text: &str) -> Result<Vec<PackageRequest>, String> {
    let mut holds = Vec::new();
    for line in text.lines() {
        let selector = line.trim();
        if selector.is_empty() {
            continue;
        }
        let entry = PackageRequest::parse(selector).map_err(|error| {
            format!(
                "apt: apt-mark showhold returned an invalid package selector {selector:?} ({error})"
            )
        })?;
        if !holds.contains(&entry) {
            holds.push(entry);
        }
    }
    Ok(holds)
}

/// One `apt-cache policy` stanza: the default candidate and every version in
/// the version table.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PolicyEntry {
    pub candidate: Option<String>,
    pub versions: Vec<String>,
}

/// Stanzas from one batched `apt-cache policy` call, keyed by the stanza
/// header (`name` for native and `all` packages, `name:arch` otherwise).
pub(super) fn parse_policy(text: &str) -> Result<HashMap<String, PolicyEntry>, String> {
    let mut entries: HashMap<String, PolicyEntry> = HashMap::new();
    let mut header: Option<&str> = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && !line.is_empty() {
            let name = line
                .strip_suffix(':')
                .ok_or_else(|| format!("apt: unrecognized policy line {line:?}"))?;
            if entries
                .insert(name.to_owned(), PolicyEntry::default())
                .is_some()
            {
                return Err(format!("apt: duplicate policy stanza for {name:?}"));
            }
            header = Some(name);
            continue;
        }
        let Some(name) = header else {
            continue;
        };
        let entry = entries.get_mut(name).ok_or("apt: policy stanza lost")?;
        if let Some(value) = line.trim().strip_prefix("Candidate: ") {
            if entry.candidate.is_some() {
                return Err(format!("apt: duplicate policy candidate for {name:?}"));
            }
            entry.candidate = (value != "(none)").then(|| value.to_owned());
        } else if let Some(row) = line
            .strip_prefix(" *** ")
            .or_else(|| line.strip_prefix("     "))
            && !row.starts_with(' ')
            && let Some(version) = row.split_whitespace().next()
        {
            // Version rows are indented five columns (or marked ***);
            // their source rows are indented further.
            entry.versions.push(version.to_owned());
        }
    }
    Ok(entries)
}
