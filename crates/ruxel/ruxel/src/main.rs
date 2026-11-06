mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Command, ConfigCommand};
use ruxel::exit::ExitCode;

fn main() {
    let code = match Cli::try_parse() {
        Ok(cli) => match cli.command {
            Command::Config(config) => match config.command {
                ConfigCommand::Dump { root, raw: _ } => match commands::config::dump(&root) {
                    Ok(()) => ExitCode::Success,
                    Err(error) => {
                        eprintln!("{error}");
                        ExitCode::Config
                    }
                },
            },
        },
        Err(error) => {
            let code = if error.use_stderr() {
                ExitCode::Usage
            } else {
                ExitCode::Success
            };
            if error.print().is_err() {
                ExitCode::Internal
            } else {
                code
            }
        }
    };
    std::process::exit(i32::from(code.code()));
}
