use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "ruxel", version = env!("RUXEL_VERSION"))]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Config(ConfigCmd),
    Plan(RunArgs),
    Apply(RunArgs),
    Check(RunArgs),
    Diff(RunArgs),
}
#[derive(Debug, clap::Args)]
pub(crate) struct ConfigCmd {
    #[command(subcommand)]
    pub(crate) command: ConfigCommand,
}
#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    Dump {
        root: PathBuf,
        #[arg(long)]
        raw: bool,
    },
}

#[derive(Debug, clap::Args)]
pub(crate) struct RunArgs {
    pub(crate) root: PathBuf,
    #[arg(long, value_delimiter = ',')]
    pub(crate) hosts: Vec<String>,
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=128))]
    pub(crate) jobs: u16,
    #[arg(long)]
    pub(crate) no_cache: bool,
    #[arg(long, value_delimiter = ',')]
    pub(crate) tags: Vec<String>,
    #[arg(long, default_value = "ruxel-agent")]
    pub(crate) agent: PathBuf,
    /// Run the real agent locally; use only disposable local workloads.
    #[arg(long, requires = "state_dir")]
    pub(crate) local: bool,
    /// Resolve every external lookup deterministically without running secret tools.
    #[arg(long)]
    pub(crate) synthetic_secrets: bool,
    #[arg(long, requires = "controller_state_dir")]
    pub(crate) local_agent: Option<PathBuf>,
    #[arg(long, requires = "local_agent")]
    pub(crate) controller_state_dir: Option<PathBuf>,
    #[arg(long, requires = "local")]
    pub(crate) state_dir: Option<PathBuf>,
}
