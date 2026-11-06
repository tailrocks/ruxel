use super::{BTreeSet, Context, Map, Options, Result, Value, array, bail, object};

pub(super) fn group_hosts(
    name: &str,
    groups: &Map<String, Value>,
    visiting: &mut BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    if !visiting.insert(name.to_owned()) {
        bail!("inventory group cycle at {name:?}");
    }
    let group = groups
        .get(name)
        .with_context(|| format!("unknown inventory group {name:?}"))?;
    let mut hosts = BTreeSet::new();
    for host in array(group, "hosts") {
        hosts.insert(
            host.as_str()
                .context("group host must be a string")?
                .to_owned(),
        );
    }
    for child in array(group, "children") {
        hosts.extend(group_hosts(
            child.as_str().context("group child must be a string")?,
            groups,
            visiting,
        )?);
    }
    visiting.remove(name);
    Ok(hosts)
}

pub(super) fn ordered_groups(groups: &Map<String, Value>) -> Result<Vec<String>> {
    let mut incoming: std::collections::BTreeMap<String, usize> =
        groups.keys().map(|name| (name.clone(), 0)).collect();
    for group in groups.values() {
        for child in array(group, "children") {
            let name = child.as_str().context("group child must be a string")?;
            *incoming
                .get_mut(name)
                .with_context(|| format!("unknown inventory group {name:?}"))? += 1;
        }
    }
    let mut ready: BTreeSet<_> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(name, _)| name.clone())
        .collect();
    let mut ordered = Vec::new();
    while let Some(name) = ready.pop_first() {
        for child in array(&groups[&name], "children") {
            let child = child.as_str().context("group child must be a string")?;
            let count = incoming.get_mut(child).context("unknown child group")?;
            *count -= 1;
            if *count == 0 {
                ready.insert(child.to_owned());
            }
        }
        ordered.push(name);
    }
    if ordered.len() != groups.len() {
        bail!("inventory group cycle");
    }
    Ok(ordered)
}

fn matches_glob(pattern: &str, name: &str) -> bool {
    let mut reachable = vec![false; name.chars().count() + 1];
    reachable[0] = true;
    let chars: Vec<_> = name.chars().collect();
    for pattern_char in pattern.chars() {
        let mut next = vec![false; reachable.len()];
        if pattern_char == '*' {
            next[0] = reachable[0];
            for i in 1..next.len() {
                next[i] = reachable[i] || next[i - 1];
            }
        } else {
            for i in 1..next.len() {
                next[i] = reachable[i - 1] && (pattern_char == '?' || pattern_char == chars[i - 1]);
            }
        }
        reachable = next;
    }
    reachable[chars.len()]
}

pub(super) fn expand_selectors(
    selectors: &[&str],
    hosts: &Map<String, Value>,
    groups: &Map<String, Value>,
) -> Result<BTreeSet<String>> {
    if selectors.is_empty() {
        bail!("inventory selector list is empty");
    }
    let mut selected = BTreeSet::new();
    for selector in selectors {
        let mut matches = BTreeSet::new();
        if *selector == "all" {
            matches.extend(hosts.keys().cloned());
        } else {
            for host in hosts.keys() {
                if matches_glob(selector, host) {
                    matches.insert(host.clone());
                }
            }
            for group in groups.keys() {
                if matches_glob(selector, group) {
                    matches.extend(group_hosts(group, groups, &mut BTreeSet::new())?);
                }
            }
        }
        if matches.is_empty() {
            bail!("inventory selector {selector:?} matched no hosts");
        }
        selected.extend(matches);
    }
    Ok(selected)
}

/// Expand play targets and host filters to concrete inventory hosts.
///
/// # Errors
/// Returns an error for invalid inventory or selectors that match no hosts.
pub fn selected_hosts(config: &Value, options: &Options) -> Result<Vec<String>> {
    let inventory = config.get("inventory").context("missing inventory")?;
    let hosts = object(inventory, "hosts");
    let groups = object(inventory, "groups");
    for name in groups.keys() {
        for host in group_hosts(name, &groups, &mut BTreeSet::new())? {
            if !hosts.contains_key(&host) {
                bail!("unknown inventory host {host:?}");
            }
        }
    }
    let mut selected = BTreeSet::new();
    for play in array(config, "plays") {
        let selectors = array(play, "targets")
            .iter()
            .map(|value| value.as_str().context("target must be a string"))
            .collect::<Result<Vec<_>>>()?;
        selected.extend(expand_selectors(&selectors, &hosts, &groups)?);
    }
    if !options.hosts.is_empty() {
        let filters: Vec<_> = options.hosts.iter().map(String::as_str).collect();
        let allowed = expand_selectors(&filters, &hosts, &groups)?;
        selected.retain(|host| allowed.contains(host));
    }
    Ok(selected.into_iter().collect())
}

#[cfg(test)]
#[path = "selection/tests.rs"]
mod tests;
