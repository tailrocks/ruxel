//! `lineinfile` (SEMANTICS §6): present = if `line` already present
//! anywhere, no change (Ansible's idempotence rule — even when regexp
//! also matches elsewhere); else replace the LAST regexp match; else
//! append at EOF. absent = delete matching lines.

use super::{ExecContext, params_object, str_param, write_atomic};
use regex_lite::Regex;
use serde_json::{Value, json};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("lineinfile: path required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    let line = str_param(obj, "line");
    let regexp = match str_param(obj, "regexp") {
        Some(r) => Some(Regex::new(r).map_err(|e| format!("lineinfile regexp: {e}"))?),
        None => None,
    };

    let current = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let (content, changed) = rewrite_lines(&current, state, line, regexp.as_ref())?;

    if changed && !ctx.check_mode {
        write_atomic(std::path::Path::new(path), content.as_bytes())?;
    }

    let mut result = json!({"changed": changed, "failed": false, "msg": if changed { "line changed" } else { "" }});
    if changed && ctx.diff_mode && !ctx.no_log {
        result["diff"] = json!(super::unified_diff(&current, &content));
    }
    Ok(result)
}

fn rewrite_lines(
    current: &str,
    state: &str,
    line: Option<&str>,
    regexp: Option<&Regex>,
) -> Result<(String, bool), String> {
    let had_trailing_nl = current.ends_with('\n');
    let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
    let mut changed = false;

    match state {
        "present" => {
            let line = line.ok_or("lineinfile: line required for present")?;
            if lines.iter().any(|l| l == line) {
                // Already present verbatim — idempotent, even if regexp
                // matches a different line.
            } else if let Some(re) = regexp {
                let last_match = lines.iter().rposition(|l| re.is_match(l));
                match last_match {
                    Some(idx) => {
                        lines[idx] = line.to_string();
                        changed = true;
                    }
                    None => {
                        lines.push(line.to_string());
                        changed = true;
                    }
                }
            } else {
                lines.push(line.to_string());
                changed = true;
            }
        }
        "absent" => {
            let before = lines.len();
            if let Some(re) = regexp {
                lines.retain(|l| !re.is_match(l));
            } else if let Some(line) = line {
                lines.retain(|l| l != line);
            } else {
                return Err("lineinfile: absent needs regexp or line".into());
            }
            changed = lines.len() != before;
        }
        other => {
            return Err(format!(
                "lineinfile: state {other:?} outside the closed surface"
            ));
        }
    }

    if changed {
        let mut content = lines.join("\n");
        if had_trailing_nl || !content.is_empty() {
            content.push('\n');
        }
        Ok((content, true))
    } else {
        Ok((current.to_string(), false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_line_wins_before_regexp_rewrite() {
        let re = Regex::new("^key=").unwrap();
        let current = "key=old\nkey=want\n";
        assert_eq!(
            rewrite_lines(current, "present", Some("key=want"), Some(&re)).unwrap(),
            (current.into(), false)
        );
    }

    #[test]
    fn replaces_last_appends_and_deletes() {
        let re = Regex::new("^key=").unwrap();
        assert_eq!(
            rewrite_lines("key=1\nother\nkey=2\n", "present", Some("key=3"), Some(&re))
                .unwrap()
                .0,
            "key=1\nother\nkey=3\n"
        );
        assert_eq!(
            rewrite_lines("other\n", "present", Some("key=3"), Some(&re))
                .unwrap()
                .0,
            "other\nkey=3\n"
        );
        assert_eq!(
            rewrite_lines("key=1\nother\n", "absent", None, Some(&re))
                .unwrap()
                .0,
            "other\n"
        );
    }

    #[test]
    fn diff_reports_changed_lines_and_stays_absent_when_unchanged() {
        let path =
            std::env::temp_dir().join(format!("ruxel-lineinfile-diff-{}", std::process::id()));
        std::fs::write(&path, "key=old\n").unwrap();
        let context = ExecContext {
            check_mode: true,
            diff_mode: true,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let changed = run(
            &json!({"path": path, "regexp": "^key=", "line": "key=new"}),
            &context,
        )
        .unwrap();
        assert!(changed["diff"].as_str().unwrap().contains("+key=new"));
        let stable = run(
            &json!({"path": path, "regexp": "^key=", "line": "key=old"}),
            &context,
        )
        .unwrap();
        assert!(stable.get("diff").is_none());
        std::fs::remove_file(path).unwrap();
    }
}
