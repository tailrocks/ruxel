use super::*;

#[test]
fn changed_workload_is_rejected_before_cli_launch() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let directory = temp.path().join("testdata/workloads/pr66");
    fs::create_dir_all(&directory)?;
    fs::write(directory.join("check-semantics.pkl"), "unreviewed fixture")?;
    let error = check_semantics(
        temp.path(),
        Path::new("/not-a-real-cli"),
        Path::new("/not-a-real-agent"),
    );
    ensure!(
        error.is_err_and(|error| error.to_string().contains("fixture changed")),
        "changed fixture was not rejected before launch"
    );
    Ok(())
}

#[test]
fn task_and_recap_checks_reject_missing_and_duplicate_events() {
    let valid = serde_json::json!({"host":"fixture","task":"task","result":{"msg":"ok"}});
    assert!(result(std::slice::from_ref(&valid), "task").is_ok());
    assert!(result(&[valid.clone(), valid], "task").is_err());
    assert!(recap(&[]).is_err());
}
