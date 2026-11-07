use super::lex;
use anyhow::{Context, Result, ensure};
use regex::Regex;
pub fn check(source: &str) -> Result<()> {
    let source = lex::strip(source)?;
    let code = lex::mask(&source)?;
    let function = Regex::new(r"(?s)async\s+fn\s+execute\s*\((.*)")?;
    let body = function
        .captures(&code)
        .context("retry entry point not found")?;
    let body = body.get(1).context("retry body not found")?;
    let body_start = body.start();
    let body = body.as_str();
    let loop_re = Regex::new(
        r"(?s)for\s+attempt\s+in\s+1\s*\.\.\s*=\s*attempts\s*\{(.*?)let\s+current_scope\s*=\s*super\s*::\s*result_scope\s*\(",
    )?;
    let retry_loop = loop_re.captures(body).context("retry loop not found")?;
    let call = Regex::new(r"self\s*\.\s*execute_once\s*\(")?;
    let direct = Regex::new(
        r#"(?s)let\s+Some\s*\(\s*condition\s*\)\s*=\s*task\s*\.\s*get\s*\(\s*("[ ]*")\s*\).*?else\s*\{\s*return\s+self\s*\.\s*execute_once\s*\("#,
    )?;
    let direct = direct
        .captures(&body[..retry_loop.get(0).context("retry loop")?.start()])
        .context("single-attempt route not found")?;
    let key = direct.get(1).context("missing condition selector")?.range();
    ensure!(
        &source[body_start + key.start..body_start + key.end] == "\"until\"",
        "single-attempt route uses the wrong selector"
    );
    ensure!(
        call.is_match(&retry_loop[1]),
        "retry-loop execution not found"
    );
    ensure!(
        Regex::new(r"(?s)\.\s*evaluate_tracked\s*\(\s*condition\s*,\s*&current_scope\s*\)")?
            .is_match(&body[retry_loop.get(0).context("retry loop")?.end()..]),
        "retry condition evaluation not found"
    );
    Ok(())
}
