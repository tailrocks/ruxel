mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Command, ConfigCommand, SchemaCommand};
use ruxel::exit::ExitCode;
use std::io::Write;

fn main() {
    let code = match Cli::try_parse() {
        Ok(cli) => match cli.command {
            Command::Plan(args) => commands::run::run(args, ruxel_engine::controller::Mode::Plan),
            Command::Apply(args) => commands::run::run(args, ruxel_engine::controller::Mode::Apply),
            Command::Check(args) => commands::run::run(args, ruxel_engine::controller::Mode::Check),
            Command::Diff(args) => commands::run::run(args, ruxel_engine::controller::Mode::Diff),
            Command::Config(config) => match config.command {
                ConfigCommand::Dump { root, sources } => {
                    match commands::config::dump(&root, &sources.options()) {
                        Ok(()) => ExitCode::Success,
                        Err(error) => {
                            if writeln!(std::io::stderr().lock(), "{error}").is_err()
                                || !error.is::<ruxel_config::ConfigError>()
                            {
                                ExitCode::Internal
                            } else {
                                ExitCode::Config
                            }
                        }
                    }
                }
            },
            Command::Schema(schema) => match schema.command {
                SchemaCommand::Export { dir } => match commands::schema::export(&dir) {
                    Ok(()) => ExitCode::Success,
                    Err(error) => {
                        drop(writeln!(
                            std::io::stderr().lock(),
                            "schema export failed: {error}"
                        ));
                        ExitCode::Internal
                    }
                },
            },
        },
        Err(error) => commands::usage::report(&error),
    };
    std::process::exit(i32::from(code.code()));
}
