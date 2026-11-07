use super::{Value, array};

pub(super) fn selected(task: &Value, inherited: &[String], filter: &[String]) -> bool {
    let mut tags = inherited.to_vec();
    tags.extend(
        array(task, "tags")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    if tags
        .iter()
        .any(|tag| tag == "always" || filter.contains(tag))
    {
        return true;
    }
    !array(task, "block").is_empty()
        && ["block", "rescue", "always"].iter().any(|section| {
            array(task, section)
                .iter()
                .any(|child| selected(child, &tags, filter))
        })
}
