use super::lex;
use anyhow::{Context, Result, ensure};
use regex::Regex;
pub fn check(source: &str, variables: &str) -> Result<()> {
    let source = lex::strip(source)?;
    let code = lex::mask(&source)?;
    let function = lex::function(&code, "execute")?;
    let body = &code[function.clone()];
    let loop_re = Regex::new(r"for\s+attempt\s+in\s+1\s*\.\.\s*=\s*attempts\s*\{")?;
    let loops: Vec<_> = loop_re.find_iter(body).collect();
    ensure!(loops.len() == 1, "missing or ambiguous retry loop");
    let loop_start = loops[0].start();
    let loop_body = lex::block(body, loops[0].end() - 1)?;
    let loop_code = &body[loop_body];
    let direct = Regex::new(
        r#"(?s)let\s+Some\s*\(\s*condition\s*\)\s*=\s*task\s*\.\s*get\s*\(\s*("[ ]*")\s*\).*?else\s*\{\s*return\s+self\s*\.\s*execute_once\s*\("#,
    )?;
    let direct = direct
        .captures(&body[..loop_start])
        .context("single-attempt route not found")?;
    let key = direct.get(1).context("missing condition selector")?.range();
    ensure!(
        &source[function.start + key.start..function.start + key.end] == "\"until\"",
        "single-attempt route uses the wrong selector"
    );
    let call = Regex::new(r"self\s*\.\s*execute_once\s*\(")?
        .find(loop_code)
        .context("retry-loop execution not found")?;
    let scope = Regex::new(r"let\s+current_scope\s*=\s*super\s*::\s*result_scope\s*\(")?
        .find(&loop_code[call.end()..])
        .context("retry result scope not found")?;
    ensure!(
        Regex::new(r"(?s)self\s*\.\s*evaluate_scoped\s*\(\s*condition\s*,\s*&current_scope\s*,\s*&layer\s*\)")?
            .is_match(&loop_code[call.end() + scope.end()..]),
        "retry condition evaluation not found"
    );
    let variables = lex::strip(variables)?;
    let variables = lex::mask(&variables)?;
    let scoped = lex::sync_function(&variables, "evaluate_scoped")?;
    let body = &variables[scoped];
    ensure!(
        Regex::new(r"let\s+definitions\s*=\s*self\s*\.\s*variable_layers\s*\(\s*layer\s*\)")?.is_match(body)
            && Regex::new(r"self\s*\.\s*backend\s*\.\s*evaluate_layered_expression\s*\(\s*value\s*,\s*scope\s*,\s*&definitions\s*\)")?.is_match(body),
        "scoped expression delegation not found"
    );
    Ok(())
}
