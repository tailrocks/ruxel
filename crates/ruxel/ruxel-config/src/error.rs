use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Pkl evaluation failed for {module}: {message}", module = .module.display())]
    Pkl {
        module: PathBuf,
        message: String,
        located: Option<String>,
    },
}
