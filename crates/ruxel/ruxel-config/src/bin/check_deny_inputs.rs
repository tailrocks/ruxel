use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use toml_edit::DocumentMut;

fn verify(root: &Path, config: &str, metadata: &Value) -> Result<(), String> {
    let document = config.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    let checks = document["checks"]
        .as_array_of_tables()
        .ok_or("missing checks array")?;
    let matching: Vec<_> = checks
        .iter()
        .filter(|check| check.get("id").and_then(toml_edit::Item::as_str) == Some("cargo-deny"))
        .collect();
    if matching.len() != 1 {
        return Err("expected exactly one cargo-deny check".into());
    }
    let inputs = matching[0]
        .get("inputs")
        .and_then(toml_edit::Item::as_array)
        .ok_or("missing cargo-deny inputs")?;
    let mut declared = BTreeSet::new();
    for input in inputs {
        let path = input.as_str().ok_or("invalid cargo-deny input")?;
        if path.ends_with("/Cargo.toml") {
            declared.insert(path.to_owned());
        }
    }
    let ids = metadata["workspace_members"]
        .as_array()
        .ok_or("missing workspace members")?;
    let mut members = BTreeSet::new();
    for id in ids {
        members.insert(id.as_str().ok_or("invalid workspace member")?);
    }
    let packages = metadata["packages"].as_array().ok_or("missing packages")?;
    let mut manifests = BTreeSet::new();
    let mut found = BTreeSet::new();
    for package in packages {
        let id = package["id"].as_str().ok_or("package missing id")?;
        if !members.contains(id) {
            continue;
        }
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or("package missing manifest path")?;
        let relative = Path::new(manifest)
            .strip_prefix(root)
            .map_err(|_| "manifest outside workspace")?;
        let relative = relative.to_str().ok_or("manifest path is not UTF-8")?;
        manifests.insert(relative.to_owned());
        found.insert(id);
    }
    if found != members {
        return Err("workspace member metadata missing".into());
    }
    if manifests != declared {
        let missing: Vec<_> = manifests.difference(&declared).collect();
        let extra: Vec<_> = declared.difference(&manifests).collect();
        return Err(format!(
            "cargo-deny manifest inputs differ: missing={missing:?}, extra={extra:?}"
        ));
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .ok_or("workspace root missing")?;
    let config =
        std::fs::read_to_string(root.join(".velnor/config.toml")).map_err(|e| e.to_string())?;
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--locked", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid cargo metadata: {e}"))?;
    verify(root, &config, &metadata)?;
    println!("cargo-deny manifest inputs match workspace members");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::verify;
    use serde_json::json;
    use std::path::Path;

    const CONFIG: &str =
        "[[checks]]\nid = 'cargo-deny'\ninputs = ['Cargo.lock', 'crate/Cargo.toml']\n";

    #[test]
    fn validates_exact_member_set_and_rejects_mismatches() {
        let metadata = json!({"workspace_members":["member"], "packages":[
            {"id":"member", "manifest_path":"/work/crate/Cargo.toml"},
            {"id":"dependency", "manifest_path":"/elsewhere/Cargo.toml"}
        ]});
        assert!(verify(Path::new("/work"), CONFIG, &metadata).is_ok());
        for config in [
            CONFIG.replace("crate/Cargo.toml", "wrong/Cargo.toml"),
            format!("{CONFIG}[[checks]]\nid='cargo-deny'\ninputs=[]\n"),
            CONFIG.replace("cargo-deny", "other"),
            CONFIG.replace("'crate/Cargo.toml'", "false"),
        ] {
            assert!(verify(Path::new("/work"), &config, &metadata).is_err());
        }
        assert!(verify(Path::new("/other"), CONFIG, &metadata).is_err());
        assert!(
            verify(
                Path::new("/work"),
                CONFIG,
                &json!({"workspace_members":["member"],"packages":[]})
            )
            .is_err()
        );
    }
}
