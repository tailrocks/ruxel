use super::verify;
use serde_json::json;
use std::path::Path;

const CONFIG: &str = "[[checks]]\nid = 'cargo-deny'\ninputs = ['Cargo.lock', 'crate/Cargo.toml']\n";

#[test]
fn validates_exact_member_set_and_rejects_mismatches() {
    let metadata = json!({"workspace_members":["member"], "packages":[
        {"id":"member", "manifest_path":"/work/crate/Cargo.toml"},
        {"id":"dependency", "source":"registry+https://example.invalid", "manifest_path":"/elsewhere/Cargo.toml"}
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

#[test]
fn local_dependency_manifests_are_required_exactly() {
    let metadata = json!({"workspace_members":["member"], "packages":[
        {"id":"member", "source":null, "manifest_path":"/work/crate/Cargo.toml"},
        {"id":"vendor", "source":null, "manifest_path":"/work/vendor/library/Cargo.toml"},
        {"id":"registry", "source":"registry+https://example.invalid", "manifest_path":"/cache/library/Cargo.toml"}
    ]});
    let config = CONFIG.replace(
        "'crate/Cargo.toml'",
        "'crate/Cargo.toml', 'vendor/library/Cargo.toml'",
    );
    assert!(verify(Path::new("/work"), &config, &metadata).is_ok());
    assert!(verify(Path::new("/work"), CONFIG, &metadata).is_err());
    assert!(
        verify(
            Path::new("/work"),
            &config.replace("vendor/library/Cargo.toml", "vendor/wrong/Cargo.toml"),
            &metadata,
        )
        .is_err()
    );
    let mut outside = metadata;
    outside["packages"][1]["manifest_path"] = json!("/outside/library/Cargo.toml");
    assert!(verify(Path::new("/work"), &config, &outside).is_err());
}
