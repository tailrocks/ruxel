//! `replace` (SEMANTICS §6): multiline regexp substitution over the whole
//! file; changed iff the substitution altered content.

use super::{ExecContext, params_object, str_param};
use regex_lite::Regex;
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("replace: path required")?;
    let pattern = str_param(obj, "regexp").ok_or("replace: regexp required")?;
    let replacement = str_param(obj, "replace").unwrap_or("");

    // Ansible compiles with re.MULTILINE.
    let re = Regex::new(&format!("(?m){pattern}")).map_err(|e| format!("replace regexp: {e}"))?;
    let current = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let replacement = parse_replacement(replacement)?;
    let next = re
        .replace_all(&current, |captures: &regex_lite::Captures<'_>| {
            replacement.expand(captures)
        })
        .to_string();
    let changed = next != current;

    if changed && !ctx.check_mode {
        std::fs::write(path, next).map_err(|e| e.to_string())?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}

#[derive(Debug, PartialEq, Eq)]
enum ReplacementPart {
    Literal(String),
    Group(usize),
}

#[derive(Debug, PartialEq, Eq)]
struct AnsibleReplacement(Vec<ReplacementPart>);

impl AnsibleReplacement {
    fn expand(&self, captures: &regex_lite::Captures<'_>) -> String {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                ReplacementPart::Literal(value) => out.push_str(value),
                ReplacementPart::Group(index) => {
                    if let Some(value) = captures.get(*index) {
                        out.push_str(value.as_str());
                    }
                }
            }
        }
        out
    }
}

fn parse_replacement(input: &str) -> Result<AnsibleReplacement, String> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            literal.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            return Err("replace: trailing backslash in replacement".into());
        };
        if next.is_ascii_digit() {
            if !literal.is_empty() {
                parts.push(ReplacementPart::Literal(std::mem::take(&mut literal)));
            }
            let mut digits = next.to_string();
            while chars.peek().is_some_and(char::is_ascii_digit) {
                digits.push(chars.next().unwrap());
            }
            parts.push(ReplacementPart::Group(
                digits
                    .parse()
                    .map_err(|_| "replace: invalid numeric backreference")?,
            ));
        } else {
            literal.push(match next {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                other => {
                    return Err(format!("replace: unsupported replacement escape \\{other}"));
                }
            });
        }
    }
    if !literal.is_empty() {
        parts.push(ReplacementPart::Literal(literal));
    }
    Ok(AnsibleReplacement(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollar_is_literal_and_numeric_backrefs_expand() {
        let regex = Regex::new("(value)").unwrap();
        let replacement = parse_replacement(r"$PATH-\1").unwrap();
        assert_eq!(
            regex.replace_all("value", |captures: &regex_lite::Captures<'_>| {
                replacement.expand(captures)
            }),
            "$PATH-value"
        );
    }
}
