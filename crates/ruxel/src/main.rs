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
    fn parses_the_drop_in_plan_shape() {
        let cli = Cli::try_parse_from([
            "ruxel",
            "plan",
            "-i",
            "hosts.ini",
            "--limit",
            "titan",
            "setup-titan.yml",
        ])
        .unwrap();
        let Command::Plan(args) = cli.command else {
            panic!("expected plan subcommand");
        };
        assert_eq!(args.inventory.to_str(), Some("hosts.ini"));
        assert_eq!(args.limit.as_deref(), Some("titan"));
        assert_eq!(args.playbook.to_str(), Some("setup-titan.yml"));
    }

    #[test]
    fn plan_requires_inventory_and_playbook() {
        assert!(Cli::try_parse_from(["ruxel", "plan"]).is_err());
        assert!(Cli::try_parse_from(["ruxel", "plan", "-i", "hosts.ini"]).is_err());
    }

    #[test]
    fn parses_apply_with_check_and_tags() {
        let cli = Cli::try_parse_from([
            "ruxel",
            "apply",
            "-i",
            "hosts.ini",
            "--check",
            "--tags",
            "sentry,velnor",
            "setup-sentry.yml",
        ])
        .unwrap();
        let Command::Apply(args) = cli.command else {
            panic!("expected apply subcommand");
        };
        assert!(args.check);
        assert_eq!(args.tags, vec!["sentry", "velnor"]);
    }

    #[test]
    fn rejects_unknown_subcommand() {
        assert!(Cli::try_parse_from(["ruxel", "run", "webservers"]).is_err());
    }
}
