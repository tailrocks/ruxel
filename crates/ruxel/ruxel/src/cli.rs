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
    Schema(SchemaCmd),
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
        #[command(flatten)]
        sources: SourceArgs,
    },
}

#[derive(Debug, clap::Args)]
pub(crate) struct SchemaCmd {
    #[command(subcommand)]
    pub(crate) command: SchemaCommand,
}
#[derive(Debug, Subcommand)]
pub(crate) enum SchemaCommand {
    /// Write the Ruxel modules this binary serves under <DIR>/ruxel/, for
    /// editors and `pkl eval --module-path <DIR>`.
    Export { dir: PathBuf },
}

/// Where the configuration may read from.
#[derive(Debug, clap::Args)]
pub(crate) struct SourceArgs {
    /// Directories that serve `modulepath:` imports, like `pkl eval
    /// --module-path`. Modules under `modulepath:/ruxel/` always come from this
    /// binary.
    #[arg(long = "module-path", value_name = "DIRS", value_delimiter = ':')]
    pub(crate) module_path: Vec<PathBuf>,
    /// The directory whose files the configuration may import and read, like
    /// `pkl eval --root-dir`. Defaults to the root module's directory.
    #[arg(long = "root-dir", value_name = "DIR")]
    pub(crate) root_dir: Option<PathBuf>,
}

impl SourceArgs {
    pub(crate) fn options(&self) -> ruxel_config::EvalOptions {
        ruxel_config::EvalOptions {
            module_path: self.module_path.clone(),
            root_dir: self.root_dir.clone(),
        }
    }
}

#[derive(Debug, clap::Args)]
pub(crate) struct RunArgs {
    pub(crate) root: PathBuf,
    #[command(flatten)]
    pub(crate) sources: SourceArgs,
    #[command(flatten)]
    pub(crate) connection: ConnectionArgs,
    #[command(flatten)]
    pub(crate) presentation: PresentationArgs,
    #[arg(long, value_delimiter = ',')]
    pub(crate) hosts: Vec<String>,
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=128))]
    pub(crate) jobs: u16,
    #[arg(long)]
    pub(crate) no_cache: bool,
    #[arg(long, value_delimiter = ',')]
    pub(crate) tags: Vec<String>,
    #[arg(long, env = "RUXEL_AGENT_BIN", default_value = "ruxel-agent")]
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

#[derive(Debug, clap::Args)]
pub(crate) struct ConnectionArgs {
    #[arg(long, env = "RUXEL_SSH_KEY")]
    pub(crate) ssh_key: Option<PathBuf>,
    #[arg(long)]
    pub(crate) known_hosts: Option<PathBuf>,
    /// Accept new host keys when the invoking SSH policy permits it.
    #[arg(long)]
    pub(crate) accept_new_host_key: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum OutputFormat {
    Json,
    Human,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PresentationArgs {
    #[arg(long, value_enum, default_value = "json")]
    pub(crate) output: OutputFormat,
    /// Write redacted JSONL events to a new private file; existing files are refused.
    #[arg(long)]
    pub(crate) log_file: Option<PathBuf>,
    /// Return exit code 3 when successful execution reports changes.
    #[arg(long)]
    pub(crate) detailed_exitcode: bool,
}
