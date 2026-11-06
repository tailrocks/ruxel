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
