mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ruxel",
    version,
    about = "Rust-native automation without the YAML archaeology"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what would change, without touching anything
    Plan(commands::plan::PlanArgs),
    /// Apply the desired state to an environment
    Apply(commands::apply::ApplyArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Plan(args) => commands::plan::execute(args),
        Command::Apply(args) => commands::apply::execute(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_plan_without_environment() {
        let cli = Cli::try_parse_from(["ruxel", "plan"]).unwrap();
        let Command::Plan(args) = cli.command else {
            panic!("expected plan subcommand");
        };
        assert_eq!(args.environment, None);
    }

    #[test]
    fn parses_apply_with_environment() {
        let cli = Cli::try_parse_from(["ruxel", "apply", "prod"]).unwrap();
        let Command::Apply(args) = cli.command else {
            panic!("expected apply subcommand");
        };
        assert_eq!(args.environment.as_deref(), Some("prod"));
    }

    #[test]
    fn rejects_unknown_subcommand() {
        assert!(Cli::try_parse_from(["ruxel", "run", "webservers"]).is_err());
    }
}
