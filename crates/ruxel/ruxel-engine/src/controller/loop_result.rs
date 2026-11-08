use super::{TrackedValue, failed, flag, json};

pub(super) fn aggregate(results: Vec<TrackedValue>, source_sensitive: bool) -> TrackedValue {
    let sensitive =
        (results.is_empty() && source_sensitive) || results.iter().any(TrackedValue::is_secret);
    let changed = results.iter().any(|value| flag(&value.value, "changed"));
    let failed = results.iter().any(|value| failed(&value.value));
    let skipped = results.iter().all(|value| flag(&value.value, "skipped"));
    let empty = results.is_empty();
    let mut result = json!({"changed":changed,"failed":failed,"skipped":skipped,"results":results.into_iter().map(|value| value.value).collect::<Vec<_>>()});
    if empty {
        result["skip"]["cause"] = json!("empty_loop");
    } else {
        result["msg"] = json!(if failed {
            "One or more items failed"
        } else if skipped {
            "All items skipped"
        } else {
            "All items completed"
        });
    }
    if sensitive {
        TrackedValue::secret(result)
    } else {
        TrackedValue::public(result)
    }
}
