use super::{ExecContext, params_object, str_param, write_atomic};
use regex_lite::Regex;
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("replace: path required")?;
    let pattern = str_param(obj, "regexp").ok_or("replace: regexp required")?;
    let replacement = str_param(obj, "replace").unwrap_or("");

    let re = Regex::new(&format!("(?m){pattern}")).map_err(|e| format!("replace regexp: {e}"))?;
    let current = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let next = re.replace_all(&current, replacement).to_string();
    let changed = next != current;

    if changed && !ctx.check_mode {
        write_atomic(std::path::Path::new(path), next.as_bytes())?;
    }
    let mut result = json!({"changed": changed, "failed": false});
    if changed && ctx.diff_mode && !ctx.no_log {
        result["diff"] = json!(super::unified_diff(&current, &next));
    }
    Ok(result)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn expands_native_capture_references_and_literal_dollars() {
        let path =
            std::env::temp_dir().join(format!("ruxel-replace-captures-{}", std::process::id()));
        std::fs::write(&path, "hello world\n").unwrap();
        let context = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let params = json!({"path": path, "regexp": "(?P<first>hello) (world)", "replace": r"${first}-$2-$$-\1"});
        assert_eq!(run(&params, &context).unwrap()["changed"], true);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "hello-world-$-\\1\n"
        );
        assert_eq!(run(&params, &context).unwrap()["changed"], false);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn diff_reports_replacement() {
        let path = std::env::temp_dir().join(format!("ruxel-replace-diff-{}", std::process::id()));
        std::fs::write(&path, "old value\n").unwrap();
        let context = ExecContext {
            check_mode: true,
            diff_mode: true,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let result = run(
            &json!({"path": path, "regexp": "old", "replace": "new"}),
            &context,
        )
        .unwrap();
        assert!(result["diff"].as_str().unwrap().contains("+new value"));
        std::fs::remove_file(path).unwrap();
    }
}
