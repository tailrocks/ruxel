use super::Execution;
use crate::controller::Backend;
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};

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
    let path = execution.runtime.source_dir.join(
        source
            .as_str()
            .context("template source path must be text")?,
    );
    ensure!(
        path.extension().is_some_and(|extension| extension == "pkl"),
        "template source must be a Pkl module"
    );
    let expression = ruxel_config::eval(&path).context("evaluate typed template source")?;
    crate::model::validate_template(&expression)?;
    params.remove("src");
    params.insert("content".into(), expression);
    Ok(())
}
