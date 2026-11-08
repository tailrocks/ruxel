use super::*;

pub(super) fn check_template_confinement(
    execution: &mut Execution,
    directory: &std::path::Path,
) -> Result<()> {
    let scope = json!({"vars":{"name":"rendered-value"}})
        .as_object()
        .context("scope")?
        .clone();
    #[cfg(unix)]
    {
        let outside = directory.with_extension("outside.pkl");
        std::fs::write(&outside, "output { value = \"outside\" }")?;
        let traversal = format!(
            "../{}",
            outside
                .file_name()
                .context("outside name")?
                .to_string_lossy()
        );
        std::os::unix::fs::symlink("source.pkl", directory.join("link.pkl"))?;
        std::os::unix::fs::symlink(directory, directory.join("linkdir"))?;
        for source in [
            traversal.as_str(),
            outside.to_str().context("outside path")?,
            "link.pkl",
            "linkdir/source.pkl",
        ] {
            let mut params = json!({"src": source})
                .as_object()
                .context("params")?
                .clone();
            assert!(
                execution
                    .prepare_operation("template", &mut params, &scope)
                    .is_err(),
                "accepted {source}"
            );
        }
        std::fs::remove_file(outside)?;
    }
    let moved = directory.with_extension("moved");
    std::fs::rename(directory, &moved)?;
    std::fs::create_dir(directory)?;
    std::fs::write(
        directory.join("source.pkl"),
        "output { value = \"replacement\" }",
    )?;
    let mut pinned = json!({"src":"source.pkl"})
        .as_object()
        .context("params")?
        .clone();
    execution.prepare_operation("template", &mut pinned, &scope)?;
    assert_eq!(
        execution.evaluate(&Value::Object(pinned), &scope)?["content"],
        "rendered-value"
    );
    std::fs::remove_dir_all(moved)?;
    Ok(())
}
