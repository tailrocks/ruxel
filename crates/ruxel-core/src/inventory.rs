//! INI inventory parsing for the workload's `hosts.ini` shape
//! (docs/SEMANTICS.md §1): named hosts in groups, with
//! `ansible_ssh_host` / `ansible_ssh_user` connection variables.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    /// `ansible_ssh_host` — connection address (falls back to `name`).
    pub ssh_host: String,
    /// `ansible_ssh_user` — login user (falls back to the current convention,
    /// which the workload always sets explicitly).
    pub ssh_user: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Inventory {
    /// Hosts in file order.
    pub hosts: Vec<Host>,
    /// Group name → member host names, in file order.
    pub groups: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error(
        "inventory line {line}: unsupported syntax: {content:?} (closed surface: [group] headers and `host key=value...` lines only)"
    )]
    UnsupportedSyntax { line: usize, content: String },
    #[error(
        "inventory line {line}: unknown host variable {key:?} (closed surface: ansible_ssh_host, ansible_ssh_user)"
    )]
    UnknownHostVar { line: usize, key: String },
    #[error("inventory line {line}: malformed `key=value` pair: {pair:?}")]
    MalformedPair { line: usize, pair: String },
}

impl Inventory {
    /// Parse the INI inventory dialect the workload uses. Anything outside
    /// it — `[group:children]`, `[group:vars]`, host ranges, quoting — is a
    /// hard error by design.
    pub fn parse(content: &str) -> Result<Self, InventoryError> {
        let mut inv = Inventory::default();
        let mut current_group: Option<String> = None;

        for (idx, raw) in content.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let Some(group) = rest.strip_suffix(']') else {
                    return Err(InventoryError::UnsupportedSyntax {
                        line: line_no,
                        content: line.to_string(),
                    });
                };
                if group.contains(':') {
                    return Err(InventoryError::UnsupportedSyntax {
                        line: line_no,
                        content: line.to_string(),
                    });
                }
                let group = group.trim().to_string();
                inv.groups.entry(group.clone()).or_default();
                current_group = Some(group);
                continue;
            }

            let mut parts = line.split_whitespace();
            let name = parts.next().expect("non-empty line").to_string();
            let mut host = Host {
                ssh_host: name.clone(),
                name,
                ssh_user: None,
            };
            for pair in parts {
                let Some((key, value)) = pair.split_once('=') else {
                    return Err(InventoryError::MalformedPair {
                        line: line_no,
                        pair: pair.to_string(),
                    });
                };
                match key {
                    "ansible_ssh_host" => host.ssh_host = value.to_string(),
                    "ansible_ssh_user" => host.ssh_user = Some(value.to_string()),
                    _ => {
                        return Err(InventoryError::UnknownHostVar {
                            line: line_no,
                            key: key.to_string(),
                        });
                    }
                }
            }
            if let Some(group) = &current_group {
                inv.groups
                    .get_mut(group)
                    .expect("group inserted on header")
                    .push(host.name.clone());
            }
            inv.hosts.push(host);
        }
        Ok(inv)
    }

    pub fn host(&self, name: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.name == name)
    }

    /// Resolve a play `hosts:` pattern intersected with an optional
    /// `--limit` pattern. Supported pattern forms (the closed surface):
    /// `all`, a group name, a host name, or a comma/colon-separated list of
    /// those.
    pub fn select(&self, pattern: &str, limit: Option<&str>) -> Result<Vec<&Host>, PatternError> {
        let base = self.expand(pattern)?;
        let selected: Vec<&Host> = match limit {
            None => base,
            Some(l) => {
                let lim = self.expand(l)?;
                base.into_iter()
                    .filter(|h| lim.iter().any(|x| x.name == h.name))
                    .collect()
            }
        };
        Ok(selected)
    }

    fn expand(&self, pattern: &str) -> Result<Vec<&Host>, PatternError> {
        let mut out: Vec<&Host> = Vec::new();
        for term in pattern.split([',', ':']).filter(|t| !t.is_empty()) {
            let matches: Vec<&Host> = if term == "all" {
                self.hosts.iter().collect()
            } else if let Some(members) = self.groups.get(term) {
                members.iter().filter_map(|name| self.host(name)).collect()
            } else if let Some(h) = self.host(term) {
                vec![h]
            } else {
                return Err(PatternError::NoMatch(term.to_string()));
            };
            for h in matches {
                if !out.iter().any(|x| x.name == h.name) {
                    out.push(h);
                }
            }
        }
        Ok(out)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PatternError {
    #[error("host pattern {0:?} matches nothing in the inventory")]
    NoMatch(String),
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKLOAD_SHAPE: &str = "\
[nodes]
pegasus ansible_ssh_host=192.0.2.10 ansible_ssh_user=root
delorean ansible_ssh_host=192.0.2.11 ansible_ssh_user=root
titan ansible_ssh_host=192.0.2.12 ansible_ssh_user=root
";

    #[test]
    fn parses_workload_shape() {
        let inv = Inventory::parse(WORKLOAD_SHAPE).unwrap();
        assert_eq!(inv.hosts.len(), 3);
        assert_eq!(inv.groups["nodes"].len(), 3);
        let pegasus = inv.host("pegasus").unwrap();
        assert_eq!(pegasus.ssh_host, "192.0.2.10");
        assert_eq!(pegasus.ssh_user.as_deref(), Some("root"));
    }

    #[test]
    fn select_all_and_group_and_host() {
        let inv = Inventory::parse(WORKLOAD_SHAPE).unwrap();
        assert_eq!(inv.select("all", None).unwrap().len(), 3);
        assert_eq!(inv.select("nodes", None).unwrap().len(), 3);
        assert_eq!(inv.select("titan", None).unwrap().len(), 1);
    }

    #[test]
    fn select_with_limit_intersects() {
        let inv = Inventory::parse(WORKLOAD_SHAPE).unwrap();
        let picked = inv.select("nodes", Some("delorean")).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].name, "delorean");
    }

    #[test]
    fn unknown_pattern_is_an_error() {
        let inv = Inventory::parse(WORKLOAD_SHAPE).unwrap();
        assert_eq!(
            inv.select("webservers", None).unwrap_err(),
            PatternError::NoMatch("webservers".into())
        );
    }

    #[test]
    fn unknown_host_var_is_a_hard_error() {
        let err = Inventory::parse("[nodes]\nh1 ansible_port=2222\n").unwrap_err();
        assert!(matches!(err, InventoryError::UnknownHostVar { key, .. } if key == "ansible_port"));
    }

    #[test]
    fn group_children_syntax_is_rejected() {
        let err = Inventory::parse("[nodes:children]\nweb\n").unwrap_err();
        assert!(matches!(err, InventoryError::UnsupportedSyntax { .. }));
    }
}
