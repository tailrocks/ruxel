mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Command, ConfigCommand, SchemaCommand};
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
                ConfigCommand::Dump { root, module_path } => {
                    match commands::config::dump(&root, &module_path.dirs) {
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
        Err(error) => {
            let code = if error.use_stderr() {
                ExitCode::Usage
            } else {
                ExitCode::Success
            };
            let version = error.kind() == clap::error::ErrorKind::DisplayVersion;
            if error.print().is_err()
                || (version
                    && writeln!(
                        std::io::stdout().lock(),
                        "guard: {} compiled deny entries",
                        ruxel_guard::Policy::compiled_entries()
                    )
                    .is_err())
            {
                ExitCode::Internal
            } else {
                code
            }
        }
    };
    std::process::exit(i32::from(code.code()));
}

fn run(args: cli::RunArgs, mode: ruxel_engine::controller::Mode) -> ExitCode {
    let value = match commands::config::admit(&args.root, &args.module_path.dirs) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let Ok(value) = ruxel_engine::model::validate_config(&value) else {
        if writeln!(std::io::stderr().lock(), "configuration validation failed").is_err() {
            return ExitCode::Internal;
        }
        return ExitCode::Config;
    };
    let options = ruxel_engine::controller::Options {
        mode,
        hosts: args.hosts,
        tags: args.tags,
    };
    let runtime_options = ruxel_engine::runtime::RuntimeOptions {
        agent: args.agent,
        jobs: usize::from(args.jobs),
        no_cache: args.no_cache,
        connection: ruxel_engine::transport::ConnectOptions {
            keyfile: args.connection.ssh_key,
            known_hosts_file: args.connection.known_hosts,
            accept_new_host_key: args.connection.accept_new_host_key,
            ..Default::default()
        },
        output: match args.presentation.output {
            cli::OutputFormat::Json => ruxel_engine::runtime::OutputFormat::Json,
            cli::OutputFormat::Human => ruxel_engine::runtime::OutputFormat::Human,
        },
        log_file: args.presentation.log_file,
        source_dir: source_dir(&args.root).to_path_buf(),
        synthetic_secrets: args.synthetic_secrets,
        local_state: args.state_dir,
        local_agent: args.local_agent,
        controller_state: args.controller_state_dir,
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
            let unreachable = recaps.values().any(|r| r.unreachable != 0);
            let failed = recaps.values().any(|r| r.failed != 0);
            let changed = recaps.values().any(|r| r.changed != 0 || r.planned != 0);
            if unreachable {
                ExitCode::Unreachable
            } else if failed {
                ExitCode::Failed
            } else if changed && args.presentation.detailed_exitcode {
                ExitCode::Changes
            } else {
                ExitCode::Success
            }
        }
        Err(error) => {
            drop(writeln!(std::io::stderr().lock(), "{error}"));
            ExitCode::Internal
        }
    }
}

fn source_dir(root: &std::path::Path) -> &std::path::Path {
    root.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
}

#[cfg(test)]
mod tests {
    #[test]
    fn bare_config_uses_current_directory_for_sources() {
        use std::path::Path;
        assert_eq!(super::source_dir(Path::new("config.pkl")), Path::new("."));
        assert_eq!(
            super::source_dir(Path::new("dir/config.pkl")),
            Path::new("dir")
        );
    }
}
