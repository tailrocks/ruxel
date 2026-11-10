use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Pkl evaluation failed for {module}: {message}", module = .module.display())]
    Pkl {
        module: PathBuf,
        message: String,
        located: Option<String>,
    },
    #[error("{}", schema_message(.found.as_deref(), *.copy, .expected))]
    Schema {
        /// The fingerprint the configuration carried, if recognizable.
        found: Option<String>,
        /// Whether that was this schema, imported from a copy.
        copy: bool,
        expected: String,
    },
}

fn schema_message(found: Option<&str>, copy: bool, expected: &str) -> String {
    let uri = crate::schema::MODULE_PREFIX;
    let fix = format!(
        "import Ruxel modules from \"{uri}\" (workflows: \"{uri}Workflow.pkl\"; engine configurations: \"{engine}\") instead of copies, and regenerate editor copies with `ruxel schema export <dir>`",
        engine = crate::schema::ENGINE_URI
    );
    match found {
        Some(_) if copy => format!(
            "configuration was evaluated against a copy of this ruxel's schema ({expected}), such as an export imported by path, not the module ruxel serves; {fix}"
        ),
        Some(found) => format!(
            "configuration was evaluated against Ruxel schema {found}, but this ruxel serves schema {expected}; {fix}"
        ),
        None => format!(
            "configuration has no Ruxel schema fingerprint, so it is not an R.Config from the schema this ruxel serves ({expected}); {fix}"
        ),
    }
}
