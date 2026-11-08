use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub(super) fn required(vars: &Map<String, Value>, expression: &Value) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    dependencies(expression, "item", false, &mut false, &mut names);
    if references_all_vars(expression) {
        names.extend(vars.keys().cloned());
    }
    loop {
        let previous = names.len();
        for name in names.clone() {
            if let Some(value) = vars.get(&name) {
                dependencies(value, "item", false, &mut false, &mut names);
                if references_all_vars(value) {
                    names.extend(vars.keys().cloned());
                }
            }
        }
        if previous == names.len() {
            return names;
        }
    }
}

fn references_all_vars(value: &Value) -> bool {
    match value {
        Value::Object(object) if object.get("kind").and_then(Value::as_str) == Some("ref") => {
            object
                .get("path")
                .and_then(Value::as_array)
                .is_some_and(|path| path.len() == 1 && matches!(path[0].as_str(), Some("vars")))
        }
        Value::Object(object) => object.values().any(references_all_vars),
        Value::Array(items) => items.iter().any(references_all_vars),
        _ => false,
    }
}

fn dependencies(
    value: &Value,
    loop_var: &str,
    item_shadowed: bool,
    direct: &mut bool,
    names: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) if object.get("kind").and_then(Value::as_str) == Some("ref") => {
            if let Some(path) = object.get("path").and_then(Value::as_array) {
                let root = path.first().and_then(Value::as_str);
                if (root == Some("item") && !item_shadowed)
                    || (loop_var != "item" && root == Some(loop_var))
                {
                    *direct = true;
                }
                if matches!(root, Some("vars"))
                    && let Some(name) = path.get(1).and_then(Value::as_str)
                {
                    names.insert(name.to_owned());
                }
            }
        }
        Value::Object(object)
            if object.get("kind").and_then(Value::as_str) == Some("call")
                && object.get("function").and_then(Value::as_str) == Some("map") =>
        {
            if let Some(args) = object.get("args").and_then(Value::as_array) {
                for (index, argument) in args.iter().enumerate() {
                    dependencies(
                        argument,
                        loop_var,
                        item_shadowed || index == 1,
                        direct,
                        names,
                    );
                }
            }
        }
        Value::Object(object) => {
            for child in object.values() {
                dependencies(child, loop_var, item_shadowed, direct, names);
            }
        }
        Value::Array(items) => {
            for child in items {
                dependencies(child, loop_var, item_shadowed, direct, names);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependencies_use_only_the_canonical_vars_root() {
        let vars = serde_json::json!({"value":1});
        let vars = vars.as_object().expect("vars");
        for tail in [vec![], vec!["value"]] {
            let mut canonical = vec!["vars"];
            canonical.extend(tail.clone());
            let mut alias = vec!["variables"];
            alias.extend(tail);
            assert_eq!(
                required(vars, &serde_json::json!({"kind":"ref","path":canonical})),
                BTreeSet::from(["value".into()])
            );
            assert!(required(vars, &serde_json::json!({"kind":"ref","path":alias})).is_empty());
        }
    }
}
