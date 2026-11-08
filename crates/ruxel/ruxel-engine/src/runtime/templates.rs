use super::Execution;
use crate::controller::Backend;
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::Path;

pub(super) type SourceRoot = ruxel_config::TemplateRoot;

pub(super) fn prepare(
    execution: &mut Execution,
    kind: &str,
    params: &mut Map<String, Value>,
    scope: &Map<String, Value>,
) -> Result<()> {
    if kind != "template" || params.get("content").is_some_and(|v| !v.is_null()) {
        return Ok(());
    }
    let source = params
        .get("src")
        .context("template requires src or content")?
        .clone();
    let source = execution.evaluate(&source, scope)?;
    let source = Path::new(
        source
            .as_str()
            .context("template source path must be text")?,
    );
    let expression = execution
        .template_root
        .evaluate(source)
        .context("evaluate typed template source")?;
    crate::model::validate_template(&expression)?;
    params.remove("src");
    params.insert("content".into(), expression);
    Ok(())
}
