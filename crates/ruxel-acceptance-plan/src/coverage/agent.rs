use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::collections::BTreeSet;
pub fn dispatch(source: &str) -> Result<BTreeSet<String>> {
    let re = Regex::new(r"(?s)let result = match module\s*\{(.*?)\n\s*_\s*=>")?;
    let region = re
        .captures(source)
        .context("agent dispatch match not found")?;
    let arms = Regex::new(r#"(?m)^\s*((?:"[^"]+"\s*(?:\|\s*)?)+)\s*=>"#)?;
    let literal = Regex::new(r#""([^"]+)""#)?;
    let kinds: BTreeSet<_> = arms
        .captures_iter(&region[1])
        .flat_map(|arm| {
            literal
                .captures_iter(arm.get(1).unwrap().as_str())
                .map(|m| m[1].to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    ensure!(
        !kinds.is_empty(),
        "agent dispatch contains no recognizable arms"
    );
    Ok(kinds)
}
pub fn admission(source: &str) -> Result<Option<BTreeSet<String>>> {
    if !Regex::new(r"\bfn\s+is_implemented\s*\(")?.is_match(source) {
        ensure!(
            !Regex::new(r"\bis_implemented\s*\(")?.is_match(source),
            "dispatch admission function not found"
        );
        return Ok(None);
    }
    let re = Regex::new(
        r"(?s)fn\s+is_implemented\s*\([^)]*\)[^{]*\{\s*matches!\s*\(\s*module\s*,(.*?)\)\s*\}",
    )?;
    let region = re
        .captures(source)
        .context("unrecognized agent admission list")?;
    let kinds: BTreeSet<_> = Regex::new(r#""([^"]+)""#)?
        .captures_iter(&region[1])
        .map(|m| m[1].to_owned())
        .collect();
    ensure!(!kinds.is_empty(), "empty agent admission list");
    Ok(Some(kinds))
}
