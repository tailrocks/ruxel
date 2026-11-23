use super::{
    ExecContext, params_object, refuse_final_symlink, reject_empty_span, str_param, write_atomic,
};
use regex_lite::Regex;
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("replace: path required")?;
    let pattern = str_param(obj, "regexp").ok_or("replace: regexp required")?;
    let replacement = str_param(obj, "replace").unwrap_or("");

    let re = Regex::new(&format!("(?m){pattern}")).map_err(|e| format!("replace regexp: {e}"))?;
    reject_empty_span("replace", pattern, re.is_match(""))?;
    let target = refuse_final_symlink("replace", path)?;
    let current = std::fs::read_to_string(target).map_err(|e| format!("read {path}: {e}"))?;
    let next = re.replace_all(&current, replacement).to_string();
    if next != current && re.replace_all(&next, replacement) != next {
        return Err(format!(
            "replace regexp {pattern:?} does not converge: replacing the result changes it again, so the task would report a change on every apply"
        ));
    }
    let changed = next != current;

    if changed && !ctx.check_mode {
        write_atomic(target, next.as_bytes())?;
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
    fn empty_span_patterns_are_refused_before_touching_the_file() {
        let path = std::env::temp_dir().join(format!("ruxel-replace-empty-{}", std::process::id()));
        std::fs::write(&path, "a\n").unwrap();
        let context = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let error = run(
            &json!({"path": path, "regexp": "^", "replace": "#"}),
            &context,
        )
        .unwrap_err();
        assert!(error.contains("empty spans"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\n");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn self_reproducing_replacements_are_refused() {
        let path =
            std::env::temp_dir().join(format!("ruxel-replace-doubling-{}", std::process::id()));
        std::fs::write(&path, "x\n").unwrap();
        let context = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let error = run(
            &json!({"path": path, "regexp": "x", "replace": "xx"}),
            &context,
        )
        .unwrap_err();
        assert!(error.contains("does not converge"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x\n");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn context_dependent_zero_width_matches_are_refused() {
        let path = std::env::temp_dir().join(format!("ruxel-replace-word-{}", std::process::id()));
        std::fs::write(&path, "a\n").unwrap();
        let context = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let error = run(
            &json!({"path": path, "regexp": "\\b", "replace": "#"}),
            &context,
        )
        .unwrap_err();
        assert!(error.contains("does not converge"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\n");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn final_symlinks_are_refused_loudly() {
        use std::os::unix::fs::symlink;
        let target =
            std::env::temp_dir().join(format!("ruxel-replace-target-{}", std::process::id()));
        let link = std::env::temp_dir().join(format!("ruxel-replace-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::fs::write(&target, "old value\n").unwrap();
        symlink(&target, &link).unwrap();
        let context = ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        let error = run(
            &json!({"path": link, "regexp": "old", "replace": "new"}),
            &context,
        )
        .unwrap_err();
        assert!(error.contains("symlink"), "{error}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old value\n");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_file(&target).unwrap();
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
