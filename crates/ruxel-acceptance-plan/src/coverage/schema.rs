use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::collections::BTreeSet;
pub fn kinds(source: &str) -> Result<BTreeSet<String>> {
    let alias = Regex::new(r"(?m)^typealias\s+Operation\s*=\s*([^\n]+)")?
        .captures(source)
        .context("Operation union not found")?;
    let names: Vec<_> = alias[1].split('|').map(str::trim).collect();
    let identifier = Regex::new(r"^[A-Za-z_]\w*$")?;
    ensure!(
        !names.is_empty() && names.iter().all(|name| identifier.is_match(name)),
        "unrecognized Operation union"
    );
    ensure!(
        names.iter().collect::<BTreeSet<_>>().len() == names.len(),
        "duplicate Operation class"
    );
    let mut kinds = BTreeSet::new();
    for name in names {
        let pattern = format!(
            r#"\bclass\s+{}\s*\{{\s*fixed\s+kind\s*=\s*"([^"]+)""#,
            regex::escape(name)
        );
        let matches: Vec<_> = Regex::new(&pattern)?.captures_iter(source).collect();
        ensure!(
            matches.len() == 1,
            "{name} must have one fixed literal kind"
        );
        ensure!(
            kinds.insert(matches[0][1].to_owned()),
            "duplicate operation kind"
        );
    }
    Ok(kinds)
}
