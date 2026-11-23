//! The plan/apply/check/diff run: admission, validation, and the
//! runtime invocation the command reports as an exit code.

use crate::cli::{OutputFormat, RunArgs};
use ruxel::exit::ExitCode;
use std::io::Write;

pub(crate) fn run(args: RunArgs, mode: ruxel_engine::controller::Mode) -> ExitCode {
    let value = match super::config::admit(&args.root, &args.sources.options()) {
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
    // The ledger identity comes from the workflow itself, never the
    // checkout's location: the same workflow from another directory
    // owns the same records (see `workflow_namespace`).
    let Some(workflow_namespace) = ruxel_engine::model::workflow_namespace(&value) else {
        if writeln!(std::io::stderr().lock(), "configuration validation failed").is_err() {
            return ExitCode::Internal;
        }
        return ExitCode::Config;
    };
    let runtime_options = ruxel_engine::runtime::RuntimeOptions {
        agent: args.agent,
        jobs: usize::from(args.jobs),
        no_cache: args.no_cache,
        workflow_namespace,
        connection: ruxel_engine::transport::ConnectOptions {
            keyfile: args.connection.ssh_key,
            known_hosts_file: args.connection.known_hosts,
            accept_new_host_key: args.connection.accept_new_host_key,
            ..Default::default()
        },
        output: match args.presentation.output {
            OutputFormat::Json => ruxel_engine::runtime::OutputFormat::Json,
            OutputFormat::Human => ruxel_engine::runtime::OutputFormat::Human,
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
mod namespace_tests {
    use super::super::config;

    /// Threads 304 and 306: the namespace derives from the admitted
    /// configuration, never the checkout — the same workflow file
    /// under two roots owns one ledger identity, and the identity
    /// carries no path for the remote agent to persist.
    #[test]
    fn same_workflow_under_two_roots_shares_its_namespace() {
        let first = std::env::temp_dir().join("ruxel-namespace-first");
        let second = std::env::temp_dir().join("ruxel-namespace-second");
        for root in [&first, &second] {
            drop(std::fs::remove_dir_all(root));
            std::fs::create_dir_all(root).expect("fixture root");
            std::fs::write(
                root.join("workflow.pkl"),
                "amends \"modulepath:/ruxel/Workflow.pkl\"\nname = \"synthetic-workflow\"\nstages {}\n",
            )
            .expect("fixture workflow");
        }
        let load = |root: &std::path::Path| {
            let options = ruxel_config::EvalOptions::default();
            let value =
                config::admit(&root.join("workflow.pkl"), &options).expect("fixture admits");
            ruxel_engine::model::validate_config(&value).expect("fixture validates")
        };
        let (first_value, second_value) = (load(&first), load(&second));
        let (first_ns, second_ns) = (
            ruxel_engine::model::workflow_namespace(&first_value).expect("fixture names"),
            ruxel_engine::model::workflow_namespace(&second_value).expect("fixture names"),
        );
        assert_eq!(first_ns, second_ns, "one workflow is one ledger owner");
        assert!(
            first_ns.chars().all(|c| c.is_ascii_hexdigit()) && !first_ns.contains('/'),
            "the namespace is an opaque digest, not a path: {first_ns}"
        );
        drop(std::fs::remove_dir_all(&first));
        drop(std::fs::remove_dir_all(&second));
    }
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
