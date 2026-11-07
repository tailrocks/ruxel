use serde_json::Value;
use std::collections::BTreeSet;

/// Detect syntactically possible delegates whose state has not been checked.
/// This deliberately does not evaluate expressions, including in skipped tasks.
pub(super) fn has_unchecked_delegates(config: &Value, checked: &BTreeSet<String>) -> bool {
    fn tasks(value: &Value, checked: &BTreeSet<String>) -> bool {
        value.as_array().is_some_and(|items| {
            items.iter().any(|task| {
                task.get("delegateTo")
                    .filter(|value| !value.is_null())
                    .is_some_and(|delegate| {
                        delegate.as_str().is_none_or(|host| !checked.contains(host))
                    })
                    || ["block", "rescue", "always"]
                        .iter()
                        .any(|key| task.get(key).is_some_and(|value| tasks(value, checked)))
            })
        })
    }
    config
        .get("plays")
        .and_then(Value::as_array)
        .is_some_and(|plays| {
            plays.iter().any(|play| {
                ["preTasks", "tasks", "handlers"]
                    .iter()
                    .any(|key| play.get(key).is_some_and(|value| tasks(value, checked)))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_dynamic_and_unchecked_nested_delegates_without_evaluation() {
        let checked = BTreeSet::from(["known".to_owned()]);
        assert!(!has_unchecked_delegates(
            &json!({"plays":[{"tasks":[{"delegateTo":"known"}]}]}),
            &checked
        ));
        for section in ["preTasks", "tasks", "handlers"] {
            let config = json!({"plays":[{section:[{"block":[{"rescue":[{"always":[{"delegateTo":{"kind":"lookup","provider":"synthetic"}}]}]}]}]}]});
            assert!(has_unchecked_delegates(&config, &checked));
        }
        assert!(has_unchecked_delegates(
            &json!({"plays":[{"tasks":[{"delegateTo":"other"}]}]}),
            &checked
        ));
        assert!(!has_unchecked_delegates(
            &json!({"plays":[{"tasks":[{"delegateTo":null}]}]}),
            &checked
        ));
    }
}
