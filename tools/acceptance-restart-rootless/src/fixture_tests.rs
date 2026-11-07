use super::*;
#[test]
fn original_pins_and_only_rootless_ownership_substitution() -> Result<()> {
    let source = include_bytes!("../../../testdata/workloads/pr66/restart-actions.pkl");
    let contract = serde_json::from_str(include_str!(
        "../../acceptance-manifest/restart-contract.json"
    ))?;
    let text = variant(source, &contract, "/synthetic")?;
    ensure!(!text.contains(OWNERS) && text.contains("mode = \"0755\""));
    ensure!(
        text == std::str::from_utf8(source)?
            .replacen(OWNERS, "", 1)
            .replace("__RUN_ROOT__", "/synthetic")
    );
    ensure!(variant(b"changed", &contract, "/synthetic").is_err());
    ensure!(variant(source, &serde_json::json!({}), "/synthetic").is_err());
    ensure!(variant(source, &contract, "/quote\"").is_err());
    Ok(())
}

#[test]
fn payload_requires_conservative_absolute_shell_safe_ascii_path() -> Result<()> {
    let source = include_bytes!("../../../testdata/workloads/pr66/restart-actions.pkl");
    let contract = serde_json::from_str(include_str!(
        "../../acceptance-manifest/restart-contract.json"
    ))?;
    for path in [
        "/tmp/test_01.path-2",
        "/var/folders/fixture/T/restart/payload",
    ] {
        ensure!(
            variant(source, &contract, path).is_ok(),
            "safe path rejected"
        );
    }
    for path in [
        "",
        "relative/path",
        "-option",
        "/tmp/space here",
        "/tmp/line\nnext",
        "/tmp/tab\tpath",
        "/tmp/quote\"",
        "/tmp/single'quote",
        "/tmp/back\\slash",
        "/tmp/semicolon;command",
        "/tmp/and&command",
        "/tmp/pipe|command",
        "/tmp/redirect>file",
        "/tmp/input<file",
        "/tmp/`command`",
        "/tmp/$(command)",
        "/tmp/$variable",
        "/tmp/(subshell)",
        "/tmp/glob*",
        "/tmp/question?",
        "/tmp/[pattern]",
        "/tmp/{one,two}",
        "/tmp/~home",
        "/tmp/é",
        "/tmp/control\u{7f}",
    ] {
        ensure!(
            variant(source, &contract, path).is_err(),
            "unsafe path admitted"
        );
    }
    Ok(())
}
