use std::process::Command;

#[test]
fn dump_and_missing_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root])
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout)?,
        serde_json::json!({"name":"ruxel", "count":1})
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", "/nonexistent.pkl"])
        .output()?;
    assert_eq!(output.status.code(), Some(4));
    Ok(())
}

#[test]
fn dump_preserves_declaration_order_and_rejects_raw() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let plain = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root])
        .output()?;
    let raw = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root, "--raw"])
        .output()?;
    assert!(plain.status.success());
    assert_eq!(raw.status.code(), Some(2));
    assert_eq!(raw.stdout, b"");
    assert!(String::from_utf8(raw.stderr)?.contains("unexpected argument '--raw' found"));
    assert_eq!(
        String::from_utf8(plain.stdout)?,
        "{\n  \"name\": \"ruxel\",\n  \"count\": 1\n}\n"
    );
    Ok(())
}

#[test]
fn version_and_usage_codes() -> Result<(), Box<dyn std::error::Error>> {
    let version = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .arg("--version")
        .output()?;
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout)?,
        format!("ruxel {}\n", env!("RUXEL_VERSION"))
    );
    let usage = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump"])
        .output()?;
    assert_eq!(usage.status.code(), Some(2));
    Ok(())
}

#[cfg(unix)]
#[test]
fn unwritable_stdout_is_internal_error() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let (writer, reader) = std::os::unix::net::UnixStream::pair()?;
    drop(reader);
    let writer = std::os::fd::OwnedFd::from(writer);
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump", root])
        .stdout(writer)
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    Ok(())
}

#[test]
fn execution_rejects_untyped_output_as_config_error() -> Result<(), Box<dyn std::error::Error>> {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ruxel-config/testdata/eval/minimal.pkl"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["plan", root])
        .output()?;
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stdout, Vec::<u8>::new());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("configuration has no Ruxel schema fingerprint"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn detailed_plan_counts_only_selected_operations() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("ruxel-cli-plan-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let root = directory.join("plan.pkl");
    std::fs::write(
        &root,
        r#"import "modulepath:/ruxel/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "plan"; targets { "fixture" }; tasks { new {
  name = "deferred"; tags { "selected" }; operation = new Ruxel.Debug { msg = "fixture" }
 } } } }
} }
"#,
    )?;
    for (tag, expected, planned) in [("selected", 3, 1), ("other", 0, 0)] {
        let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
            .arg("plan")
            .arg(&root)
            .args([
                "--detailed-exitcode",
                "--tags",
                tag,
                "--agent",
                "/missing-agent",
            ])
            .output()?;
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert!(
            events
                .iter()
                .any(|event| event["recap"]["planned"] == planned)
        );
    }
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

fn scratch(name: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("ruxel-cli-{name}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&directory));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

const CONFIG: &str = r#"import "IMPORT" as R
config = new R.Config {
  inventory { hosts { ["fixture"] { address = "ADDRESS" } } }
  plays { new { name = "plan"; targets { "fixture" }; tasks { new {
    name = "deferred"; operation = new R.Debug { msg = "fixture" }
  } } } }
}
"#;

fn plan(root: &std::path::Path, extra: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .arg("plan")
        .arg(root)
        .args(["--agent", "/missing-agent"])
        .args(extra)
        .output()
}

#[test]
fn execution_reports_validation_failures_without_detail() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = scratch("validation")?;
    let root = directory.join("config.pkl");
    std::fs::write(
        &root,
        CONFIG
            .replace("IMPORT", "modulepath:/ruxel/Ruxel.pkl")
            .replace("ADDRESS", "-invalid"),
    )?;
    let output = plan(&root, &[])?;
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stderr, b"configuration validation failed\n");
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn schema_export_serves_editors_and_stale_exports_are_refused()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch("export")?;
    let export = directory.join("schema");
    let output = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["schema", "export"])
        .arg(&export)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    let exported = std::fs::read_to_string(export.join("ruxel/Ruxel.pkl"))?;
    assert!(exported.contains("fixed schemaFingerprint: String = \"blake3:"));

    // The export is what this binary serves: a configuration importing it by
    // path is admitted, and so is one using the module path URI.
    let root = directory.join("config.pkl");
    for import in ["schema/ruxel/Ruxel.pkl", "modulepath:/ruxel/Ruxel.pkl"] {
        std::fs::write(
            &root,
            CONFIG
                .replace("IMPORT", import)
                .replace("ADDRESS", "fixture.invalid"),
        )?;
        let output = plan(&root, &[])?;
        assert_eq!(output.status.code(), Some(0), "{import}: {output:?}");
    }

    // Another binary's export is refused, even when passed as --module-path,
    // because the compiled-in schema always wins modulepath:/ruxel/Ruxel.pkl.
    let fingerprint = exported
        .split("schemaFingerprint: String = \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .ok_or("no fingerprint")?
        .to_owned();
    std::fs::write(
        export.join("ruxel/Ruxel.pkl"),
        exported.replace(&fingerprint, "blake3:stale"),
    )?;
    std::fs::write(
        &root,
        CONFIG
            .replace("IMPORT", "schema/ruxel/Ruxel.pkl")
            .replace("ADDRESS", "fixture.invalid"),
    )?;
    let output = plan(&root, &[])?;
    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("blake3:stale"), "{stderr}");
    assert!(stderr.contains(&fingerprint), "{stderr}");
    assert!(stderr.contains("modulepath:/ruxel/Ruxel.pkl"), "{stderr}");
    std::fs::write(
        &root,
        CONFIG
            .replace("IMPORT", "modulepath:/ruxel/Ruxel.pkl")
            .replace("ADDRESS", "fixture.invalid"),
    )?;
    let module_path = export.display().to_string();
    let output = plan(&root, &["--module-path", &module_path])?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn module_path_serves_shared_roles() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch("module-path")?;
    let first = directory.join("first");
    let second = directory.join("second");
    std::fs::create_dir_all(first.join("lib"))?;
    std::fs::create_dir_all(second.join("roles"))?;
    std::fs::write(first.join("lib/name.pkl"), "name = \"from-first\"\n")?;
    std::fs::write(
        second.join("roles/debug.pkl"),
        "import \"modulepath:/ruxel/Ruxel.pkl\" as R\nimport \"modulepath:/lib/name.pkl\" as N\ntask = new R.Task { name = N.name; operation = new R.Debug { msg = N.name } }\n",
    )?;
    let root = directory.join("config.pkl");
    std::fs::write(
        &root,
        r#"import "modulepath:/ruxel/Ruxel.pkl" as R
import "modulepath:/roles/debug.pkl" as Debug
config = new R.Config {
  inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
  plays { new { name = "roles"; targets { "fixture" }; tasks { Debug.task } } }
}
"#,
    )?;
    let module_path = format!("{}:{}", first.display(), second.display());
    let dump = Command::new(env!("CARGO_BIN_EXE_ruxel"))
        .args(["config", "dump"])
        .arg(&root)
        .args(["--module-path", &module_path])
        .output()?;
    assert!(dump.status.success(), "{dump:?}");
    let value: serde_json::Value = serde_json::from_slice(&dump.stdout)?;
    assert_eq!(
        value["config"]["plays"][0]["tasks"][0]["name"],
        "from-first"
    );
    let output = plan(&root, &["--module-path", &module_path])?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let output = plan(&root, &[])?;
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
