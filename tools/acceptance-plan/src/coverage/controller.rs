use super::lex;
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::collections::BTreeSet;
pub fn kinds(operation: &str, local: &str) -> Result<BTreeSet<String>> {
    let operation = lex::strip(operation)?;
    let local = lex::strip(local)?;
    let code = lex::mask(&operation)?;
    let local_code = lex::mask(&local)?;
    let admission = Regex::new(
        r#"(?s)if\s+matches!\s*\(\s*kind\s*,\s*((?:"[ ]*"\s*(?:\|\s*)?)+)\)\s*\{\s*return\s+self\s*\.\s*local_retry\s*\("#,
    )?;
    let matches: Vec<_> = admission.captures_iter(&code).collect();
    ensure!(
        matches.len() == 1,
        "controller local admission route not found or ambiguous"
    );
    let range = matches[0].get(1).context("missing admission")?.range();
    let admitted_text = &operation[range];
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
    let functions = Regex::new(
        r"(?s)async\s+fn\s+local_once\s*\(.*?match\s+kind\s*\{(.*?)\n\s*_\s*=>.*?async\s+fn\s+local_retry\s*\((.*)",
    )?;
    let regions = functions
        .captures(&local_code)
        .context("controller local dispatch or retry route not found")?;
    let retry = regions.get(2).context("missing retry")?.as_str();
    let loop_re = Regex::new(
        r"(?s)for\s+attempt\s+in\s+1\s*\.\.\s*=\s*attempts\s*\{(.*?)\blet\s+scope\s*=\s*result_scope\s*\(",
    )?;
    let body = loop_re
        .captures(retry)
        .context("controller retry-loop local dispatch not found")?;
    let call = Regex::new(r"\.\s*local_once\s*\(")?;
    ensure!(
        call.is_match(&body[1]),
        "controller retry-loop local dispatch not found"
    );
    ensure!(
        call.is_match(&retry[..body.get(0).context("missing loop")?.start()]),
        "controller non-retry local dispatch not found"
    );
    let region = regions.get(1).context("missing local dispatch")?;
    let arm = Regex::new(r#"(?m)^\s*"([ ]*)"\s*=>"#)?;
    let valid = Regex::new(r"^[a-z_]+$")?;
    let mut arms = Vec::new();
    for item in arm.captures_iter(region.as_str()) {
        let range = item.get(1).context("missing arm")?.range();
        let kind = &local[region.start() + range.start..region.start() + range.end];
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
