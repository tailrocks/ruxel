use super::OutputFormat;
use crate::transport;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub agent: PathBuf,
    pub jobs: usize,
    pub no_cache: bool,
    pub connection: transport::ConnectOptions,
    pub output: OutputFormat,
    pub log_file: Option<PathBuf>,
    pub source_dir: PathBuf,
    pub synthetic_secrets: bool,
    pub local_state: Option<PathBuf>,
    pub local_agent: Option<PathBuf>,
    pub controller_state: Option<PathBuf>,
}
