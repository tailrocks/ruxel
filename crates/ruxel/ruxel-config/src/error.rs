use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Pkl evaluation failed for {module}: {message}", module = .module.display())]
    Pkl {
        module: PathBuf,
        message: String,
        located: Option<String>,
    },
    #[error("{}", schema_message(.found.as_deref(), .expected))]
    Schema {
        found: Option<String>,
        expected: String,
    },
}

fn schema_message(found: Option<&str>, expected: &str) -> String {
    let uri = crate::schema::MODULE_URI;
    let fix = format!(
        "import \"{uri}\" instead of a copy of Ruxel.pkl, and regenerate editor copies with `ruxel schema export <dir>`"
    );
    match found {
        Some(found) => format!(
            "configuration was evaluated against Ruxel schema {found}, but this ruxel serves schema {expected}; {fix}"
        ),
        None => format!(
            "configuration has no Ruxel schema fingerprint, so it is not an R.Config from the schema this ruxel serves ({expected}); {fix}"
        ),
    }
}
