use serde_json::{Map, Value};
use std::collections::BTreeSet;

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
                if matches!(root, Some("vars" | "variables"))
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

/// Variables depending on the current item, including transitive dependencies.
pub(super) fn item_dependent(vars: &Map<String, Value>, loop_var: &str) -> BTreeSet<String> {
    let mut dependent = BTreeSet::new();
    let mut graph = std::collections::BTreeMap::new();
    for (name, value) in vars {
        let mut direct = false;
        let mut names = BTreeSet::new();
        dependencies(value, loop_var, false, &mut direct, &mut names);
        if direct {
            dependent.insert(name.clone());
        }
        graph.insert(name.clone(), names);
    }
    loop {
        let previous = dependent.len();
        for (name, names) in &graph {
            if names
                .iter()
                .any(|dependency| dependent.contains(dependency))
            {
                dependent.insert(name.clone());
            }
        }
        if dependent.len() == previous {
            return dependent;
        }
    }
}
