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

impl<B: super::Backend> super::Runner<'_, B> {
    pub(super) fn skip_tagged_leaf(
        &mut self,
        task: &Value,
        parent: &super::ContextLayer,
    ) -> super::Result<()> {
        self.recap.skipped += 1;
        let result = if parent.no_log || parent.control_sensitive || super::flag(task, "noLog") {
            super::json!({"censored":true,"changed":false,"failed":false})
        } else {
            super::json!({"skipped":true,"changed":false})
        };
        self.backend.event(&super::json!({"host":self.host,"task":super::string(task,"name").unwrap_or(""),"result":result}))
    }
}
