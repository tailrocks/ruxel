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
    /// Apply-time fixture switch for local acceptance runs: resolve every
    /// lookup deterministically without running secret tools. `plan`,
    /// `check` and `diff` are always synthetic whatever this says; only
    /// `apply` reads real stores, and only `apply` generates.
    pub synthetic_secrets: bool,
    pub local_state: Option<PathBuf>,
    pub local_agent: Option<PathBuf>,
    pub controller_state: Option<PathBuf>,
}
