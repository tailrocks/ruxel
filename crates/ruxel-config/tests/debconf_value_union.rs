//! The debconf answer union of `Debian.pkl` is closed and checked: a
//! static answer, a secret answer, and nothing else.
//!
//! The union is `OneLine|SecretAnswer`, not `Static|core.SecretText`: the
//! second would name a class of another module in a property type, and
//! pklr then loses the parent class of every class of the module the
//! module path serves (see `pklr_module_path_parent.rs`). Official Pkl
//! 0.33.0 checks both spellings the same way.

fn evaluate(source: &str) -> Result<serde_json::Value, String> {
    let directory = std::env::temp_dir().join(format!(
        "ruxel-debconf-union-{}-{}",
        std::process::id(),
        source.len()
    ));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let root = directory.join("main.pkl");
    std::fs::write(&root, source).map_err(|error| error.to_string())?;
    let options = ruxel_config::EvalOptions::default();
    let result = ruxel_config::eval(&root, &options).map_err(|error| error.to_string());
    drop(std::fs::remove_dir_all(&directory));
    result
}

fn entry(value: &str) -> String {
    format!(
        "module main

import \"modulepath:/ruxel/Debian.pkl\" as debian
import \"modulepath:/ruxel/Core.pkl\" as core

step = new debian.DebconfEntry {{
  package = \"postfix\"
  question = \"postfix/mailname\"
  type = \"string\"
  value = {value}
}}
"
    )
}

#[test]
fn a_static_answer_is_admitted() {
    let value = evaluate(&entry("\"mail.example.test\"")).expect("the static answer is admitted");
    assert_eq!(value["step"]["value"], "mail.example.test");
}

#[test]
fn a_secret_answer_is_admitted() {
    let value = evaluate(&entry(
        "debian.secretAnswer(core.fnox(\"POSTFIX_RELAY_PW\"))",
    ))
    .expect("the secret answer is admitted");
    assert_eq!(value["step"]["value"], serde_json::json!({}));
}

#[test]
fn an_answer_of_another_type_is_refused() {
    for value in [
        "42",
        "true",
        "new core.File { path = \"/tmp/x\"; content = \"x\" }",
    ] {
        let error = evaluate(&entry(value)).expect_err(value);
        assert!(error.contains("value"), "{value}: {error}");
    }
}

#[test]
fn a_secret_interpolated_into_text_is_refused() {
    let error = evaluate(&entry("\"pw-\\(core.fnox(\"POSTFIX_RELAY_PW\"))\""))
        .expect_err("the interpolated secret is refused");
    assert!(error.contains("value"), "{error}");
    assert!(
        error.contains("__ruxel_secret_8d41.") || error.contains("expected"),
        "{error}"
    );
}

/// The superseded cross-module representation is gone: `DebconfEntry.value`
/// is the closed `OneLine|SecretAnswer` union, so a `core.SecretText` can
/// never reach the lowering, and the dead `core.SecretText` branches of
/// `debconfValue` and `secretStep` must not come back. The module text is
/// read through the schema the binary serves, which is the only copy.
#[test]
fn the_cross_module_secret_representation_is_dropped() {
    let lowering = ruxel_config::schema::modules()
        .iter()
        .find(|(path, _)| path.ends_with("internal/Lowering.pkl"))
        .map(|(_, text)| text)
        .expect("the served lowering module");
    assert!(
        !lowering.contains("else if (v is core.SecretText) secretValue(v.parts)"),
        "debconfValue still carries the dead core.SecretText branch"
    );
    assert!(
        !lowering.contains("s.value is core.SecretText"),
        "secretStep still carries the dead core.SecretText disjunct"
    );
}
