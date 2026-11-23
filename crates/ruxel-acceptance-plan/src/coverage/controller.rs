use super::lex;
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::collections::BTreeSet;
pub fn kinds(operation: &str, local: &str) -> Result<BTreeSet<String>> {
    let operation = lex::strip(operation)?;
    let local = lex::strip(local)?;
    let code = lex::mask(&operation)?;
    let local_code = lex::mask(&local)?;
    let operation_range = lex::function(&code, "execute_once")?;
    let code = &code[operation_range.clone()];
    let local_range = lex::function(&local_code, "local_once")?;
    let local_code = &local_code[local_range.clone()];
    let admission = Regex::new(
        r#"(?s)if\s+matches!\s*\(\s*kind\s*,\s*((?:"[ ]*"\s*(?:\|\s*)?)+)\)\s*\{\s*return\s+self\s*\.\s*local_once\s*\("#,
    )?;
    let matches: Vec<_> = admission.captures_iter(code).collect();
    ensure!(
        matches.len() == 1,
        "controller local admission route not found or ambiguous"
    );
    let range = matches[0].get(1).context("missing admission")?.range();
    let admitted_text =
        &operation[operation_range.start + range.start..operation_range.start + range.end];
    ensure!(
        Regex::new(r#"^\s*"[a-z_]+"(?:\s*\|\s*"[a-z_]+")*\s*$"#)?.is_match(admitted_text),
        "invalid controller local admission"
    );
    let literal = Regex::new(r#""([a-z_]+)""#)?;
    let admitted: Vec<_> = literal
        .captures_iter(admitted_text)
        .map(|m| m[1].to_owned())
        .collect();
    let admitted_set: BTreeSet<_> = admitted.iter().cloned().collect();
    ensure!(
        !admitted.is_empty() && admitted.len() == admitted_set.len(),
        "invalid controller local admission"
    );
    ensure!(
        Regex::new(r"(?s)let\s+result\s*=\s*self\s*\.\s*backend\s*\.\s*execute_tracked\s*\(")?
            .is_match(&code[matches[0].get(0).context("missing route")?.end()..]),
        "remote single-attempt dispatch not found"
    );
    let match_start = Regex::new(r"match\s+kind\s*\{")?
        .find(local_code)
        .context("controller local match not found")?;
    let match_range = lex::block(local_code, match_start.end() - 1)?;
    let match_code = &local_code[match_range.clone()];
    let functions = Regex::new(r"(?s)(.*?)\n\s*_\s*=>")?;
    let regions = functions
        .captures(match_code)
        .context("controller local dispatch not found")?;
    let region = regions.get(1).context("missing local dispatch")?;
    let arm = Regex::new(r#"(?m)^\s*"([ ]*)"\s*=>"#)?;
    let valid = Regex::new(r"^[a-z_]+$")?;
    let mut arms = Vec::new();
    for item in arm.captures_iter(region.as_str()) {
        let range = item.get(1).context("missing arm")?.range();
        let offset = local_range.start + match_range.start + region.start();
        let kind = &local[offset + range.start..offset + range.end];
        ensure!(
            valid.is_match(kind),
            "invalid controller local dispatch arm"
        );
        arms.push(kind.to_owned());
    }
    let result: BTreeSet<_> = arms.iter().cloned().collect();
    ensure!(
        !arms.is_empty() && arms.len() == result.len(),
        "invalid controller local dispatch"
    );
    ensure!(
        admitted_set == result,
        "controller local admission/dispatch mismatch"
    );
    Ok(result)
}
