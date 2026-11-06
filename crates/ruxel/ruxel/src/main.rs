mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Command, ConfigCommand};
use ruxel::exit::ExitCode;
use std::io::Write;

fn main() {
    let code = match Cli::try_parse() {
        Ok(cli) => match cli.command {
            Command::Plan(args) => run(args, ruxel_engine::controller::Mode::Plan),
            Command::Apply(args) => run(args, ruxel_engine::controller::Mode::Apply),
            Command::Check(args) => run(args, ruxel_engine::controller::Mode::Check),
            Command::Diff(args) => run(args, ruxel_engine::controller::Mode::Diff),
            Command::Config(config) => match config.command {
                ConfigCommand::Dump { root, raw: _ } => match commands::config::dump(&root) {
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

fn run(args: cli::RunArgs, mode: ruxel_engine::controller::Mode) -> ExitCode {
    let value = match ruxel_config::eval(&args.root) {
        Ok(value) => value,
        Err(error) => {
            let _ = writeln!(std::io::stderr().lock(), "{error}");
            return ExitCode::Config;
        }
    };
    let options = ruxel_engine::controller::Options {
        mode,
        hosts: args.hosts,
        tags: args.tags,
    };
    let runtime_options = ruxel_engine::runtime::RuntimeOptions {
        agent: args.agent,
        source_dir: args
            .root
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf(),
        synthetic_secrets: args.synthetic_secrets,
        local_state: args.state_dir,
    };
    let result = tokio::runtime::Runtime::new()
        .map_err(|error| error.to_string())
        .and_then(|runtime| {
            runtime
                .block_on(ruxel_engine::runtime::run(
                    &value,
                    &options,
                    &runtime_options,
                ))
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(recaps) => {
            let failed = recaps.values().any(|r| r.failed != 0);
            for (host, recap) in recaps {
                let event = serde_json::json!({"host":host,"recap":{"ok":recap.ok,"changed":recap.changed,"failed":recap.failed,"skipped":recap.skipped,"rescued":recap.rescued,"ignored":recap.ignored}});
                if writeln!(std::io::stdout().lock(), "{event}").is_err() {
                    return ExitCode::Internal;
                }
            }
            if failed {
                ExitCode::Failed
            } else {
                ExitCode::Success
            }
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr().lock(), "{error}");
            ExitCode::Internal
        }
    }
}
