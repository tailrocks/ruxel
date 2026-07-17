mod commands;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Plan(args) => commands::plan::execute(args),
        Command::Apply(args) => commands::apply::execute(args),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(exit_code_for_error(&error))
        }
    }
}

fn exit_code_for_error(error: &anyhow::Error) -> u8 {
    if error
        .downcast_ref::<ruxel_core::playbook::ParseError>()
        .is_some()
        || error
            .downcast_ref::<Box<ruxel_core::playbook::ParseError>>()
            .is_some()
        || error
            .downcast_ref::<ruxel_core::inventory::InventoryError>()
            .is_some()
        || error
            .downcast_ref::<ruxel_core::compiler::CompileError>()
            .is_some()
        || error
            .downcast_ref::<Box<ruxel_core::compiler::CompileError>>()
            .is_some()
    {
        2
    } else {
        1
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

    #[test]
    fn classifies_contract_errors_separately_from_runtime_errors() {
        let parse: anyhow::Error = ruxel_core::playbook::parse("bad.yml", "not-a-playbook")
            .unwrap_err()
            .into();
        assert_eq!(exit_code_for_error(&parse), 2);

        let inventory: anyhow::Error =
            ruxel_core::inventory::Inventory::parse("host unsupported=value")
                .unwrap_err()
                .into();
        assert_eq!(exit_code_for_error(&inventory), 2);

        let compile: anyhow::Error = ruxel_core::compiler::CompileError::Surface {
            playbook: "synthetic.yml".into(),
            task: "synthetic".into(),
            kind: ruxel_core::playbook::ErrorKind::NoModule,
        }
        .into();
        assert_eq!(exit_code_for_error(&compile), 2);
        assert_eq!(exit_code_for_error(&anyhow::anyhow!("host down")), 1);
    }
}
