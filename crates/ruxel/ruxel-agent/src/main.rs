mod facts;
mod ledger;
mod modules;
mod sensitivity;
mod system_state;

use ruxel_proto::PROTO_VERSION;
use ruxel_proto::frame::{read_frame, write_frame};
use ruxel_proto::v1::{self, envelope::Msg};
use std::io::Write as _;

#[expect(
    clippy::let_underscore_must_use,
    reason = "replaced by the Ruxel rewrite"
)]
fn main() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(std::string::ToString::to_string)
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic with non-string payload".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let crash = v1::Event {
            msg: Some(v1::event::Msg::Crash(v1::CrashReport { message, location })),
        };
        let mut out = std::io::stdout().lock();
        let _ = write_frame(&mut out, &crash);
        let _ = out.flush();
        std::process::exit(70);
    }));

    let code = serve();
    std::process::exit(code);
}

#[expect(
    clippy::too_many_lines,
    clippy::let_underscore_must_use,
    reason = "replaced by the Ruxel rewrite"
)]
fn serve() -> i32 {
    struct ModuleGuard;
    impl Drop for ModuleGuard {
        fn drop(&mut self) {
            modules::shutdown();
        }
    }
    let _module_guard = ModuleGuard;
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    let dir = sensitivity::state_dir();
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let lock_path = dir.join("agent.lock");
    let lock_file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            log_event(
                &mut stdout,
                v1::log::Level::Error,
                format!("lock open: {e}"),
            );
            return 66;
        }
    };
    if lock_file.try_lock().is_err() {
        log_event(
            &mut stdout,
            v1::log::Level::Error,
            "another ruxel run holds the lock".to_string(),
        );
        return 66;
    }

    let Ok(mut sensitive_effects) = sensitivity::Marker::load(&dir) else {
        log_event(
            &mut stdout,
            v1::log::Level::Error,
            "sensitivity state unavailable".to_owned(),
        );
        return 66;
    };

    let handshaken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let handshaken = handshaken.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(
                ruxel_proto::constants::HANDSHAKE_TIMEOUT_SECS,
            ));
            if !handshaken.load(std::sync::atomic::Ordering::Relaxed) {
                std::process::exit(67);
            }
        });
    }

    let mut check_mode = false;
    let mut diff_mode = false;
    let mut no_cache = false;
    let mut ledger = ledger::Ledger::load(&dir);
    let mut system_state = system_state::SystemState::default();

    loop {
        let envelope: v1::Envelope = match read_frame(&mut stdin) {
            Ok(Some(env)) => env,
            Ok(None) => {
                ledger.flush();
                return 0;
            }
            Err(e) => {
                log_event(&mut stdout, v1::log::Level::Error, format!("frame: {e}"));
                ledger.flush();
                return 64;
            }
        };
        match envelope.msg {
            Some(Msg::Hello(hello)) => {
                handshaken.store(true, std::sync::atomic::Ordering::Relaxed);
                check_mode = hello.check_mode;
                diff_mode = hello.diff_mode;
                no_cache = hello.no_cache;
                if hello.proto_version != PROTO_VERSION {
                    log_event(
                        &mut stdout,
                        v1::log::Level::Error,
                        format!(
                            "proto version mismatch: controller {} vs agent {}",
                            hello.proto_version, PROTO_VERSION
                        ),
                    );
                    return 65;
                }
                let ack = v1::Event {
                    msg: Some(v1::event::Msg::HelloAck(v1::HelloAck {
                        agent_version: env!("CARGO_PKG_VERSION").into(),
                        proto_version: PROTO_VERSION,
                        facts: Some(facts::gather()),
                        ledger_generation: 0, // ledger lands in M3
                        sensitive_effects: Some(sensitive_effects.is_marked()),
                    })),
                };
                if write_frame(&mut stdout, &ack).is_err() {
                    return 64;
                }
            }
            Some(Msg::Done(_)) => {
                ledger.flush();
                return 0;
            }
            Some(Msg::Plan(v1::Plan { tasks, .. }) | Msg::PlanPatch(v1::PlanPatch { tasks })) => {
                for task in &tasks {
                    let mut task = task.clone();
                    task.no_log |= sensitive_effects.is_marked();
                    if execute_task(
                        &mut stdout,
                        &task,
                        check_mode,
                        diff_mode,
                        no_cache,
                        &mut ledger,
                        &mut system_state,
                        &mut sensitive_effects,
                    ) {
                        break;
                    }
                }
            }
            Some(Msg::Resume(_)) => {
                log_event(
                    &mut stdout,
                    v1::log::Level::Warn,
                    "unexpected Resume outside a pause".to_string(),
                );
            }
            None => {
                log_event(
                    &mut stdout,
                    v1::log::Level::Error,
                    "empty envelope".to_string(),
                );
                return 64;
            }
        }
    }
}

#[expect(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::let_underscore_must_use,
    reason = "task execution carries protocol, state and mode together"
)]
fn execute_task(
    out: &mut impl std::io::Write,
    task: &v1::RenderedTask,
    check_mode: bool,
    diff_mode: bool,
    no_cache: bool,
    ledger: &mut ledger::Ledger,
    system_state: &mut system_state::SystemState,
    sensitive_effects: &mut sensitivity::Marker,
) -> bool {
    let task_check_mode =
        effective_check_mode(check_mode, task.force_check_mode, task.check_mode_override);
    if task.no_log && sensitive_effects.mark().is_err() {
        if let Some(iteration) = task.iterations.first() {
            send_result(
                out,
                task,
                iteration,
                "failed",
                false,
                &serde_json::json!({"failed":true,"msg":"sensitivity state unavailable"}),
                std::time::Instant::now(),
            );
        }
        return true;
    }
    for iteration in &task.iterations {
        let start = std::time::Instant::now();
        let _ = write_frame(
            out,
            &v1::Event {
                msg: Some(v1::event::Msg::TaskStart(v1::TaskStart {
                    task_id: task.task_id,
                    item_label: iteration.item_label.clone(),
                })),
            },
        );

        let parsed = if iteration.params_json.is_empty() {
            Ok(serde_json::json!({}))
        } else {
            serde_json::from_slice(&iteration.params_json).map_err(|e| e.to_string())
        };
        let params = match parsed.and_then(|params| {
            modules::validate_source(&task.module, &params, iteration.source_content.as_deref())?;
            Ok(params)
        }) {
            Ok(params) => params,
            Err(e) => {
                send_result(
                    out,
                    task,
                    iteration,
                    "failed",
                    true,
                    &serde_json::json!({"failed": true, "msg": format!("bad params: {e}")}),
                    start,
                );
                if task.halt_on_failure {
                    return true;
                }
                continue;
            }
        };

        if !no_cache
            && ledger::cache_allowed(&task.module, &params)
            && !task_check_mode
            && !task.no_log
            && !iteration.ledger_key.is_empty()
            && let Some(cached) = ledger.cached_ok_with_state(&iteration.ledger_key, system_state)
        {
            send_result(out, task, iteration, "ok", false, &cached, start);
            continue;
        }

        let ctx = modules::ExecContext {
            check_mode: task_check_mode,
            diff_mode,
            no_log: task.no_log,
            environment: task
                .environment
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            become_user: if task.become_user.is_empty() {
                None
            } else {
                Some(task.become_user.clone())
            },
        };
        let outcome = modules::execute(
            &task.module,
            &params,
            &iteration.free_form,
            iteration.source_content.as_deref(),
            &ctx,
            system_state,
        );
        if !task_check_mode && !task.no_log {
            ledger.record_with_state(
                &iteration.ledger_key,
                &task.module,
                &params,
                outcome.status,
                &outcome.result,
                system_state,
            );
        }
        send_result(
            out,
            task,
            iteration,
            outcome.status,
            outcome.changed,
            &outcome.result,
            start,
        );
        if outcome.status == "failed" && task.halt_on_failure {
            return true;
        }
    }
    false
}

fn effective_check_mode(global: bool, force: bool, override_off: bool) -> bool {
    (global || force) && !override_off
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::let_underscore_must_use,
    reason = "replaced by the Ruxel rewrite"
)]
fn send_result(
    out: &mut impl std::io::Write,
    task: &v1::RenderedTask,
    iteration: &v1::Iteration,
    status: &str,
    changed: bool,
    result: &serde_json::Value,
    start: std::time::Instant,
) {
    let _ = write_frame(
        out,
        &v1::Event {
            msg: Some(v1::event::Msg::TaskResult(v1::TaskResult {
                task_id: task.task_id,
                status: status.to_string(),
                changed,
                result_json: serde_json::to_vec(result).unwrap_or_default(),
                diff: if task.no_log {
                    String::new()
                } else {
                    result
                        .get("diff")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string()
                },
                elapsed_ms: start.elapsed().as_millis() as u64,
                item_label: iteration.item_label.clone(),
            })),
        },
    );
}

#[expect(
    clippy::let_underscore_must_use,
    reason = "replaced by the Ruxel rewrite"
)]
fn log_event(out: &mut impl std::io::Write, level: v1::log::Level, message: String) {
    let event = v1::Event {
        msg: Some(v1::event::Msg::Log(v1::Log {
            level: level as i32,
            message,
        })),
    };
    let _ = write_frame(out, &event);
}

#[cfg(test)]
mod tests {
    use super::effective_check_mode;

    #[test]
    fn task_check_mode_yes_forces_prediction_and_no_forces_execution() {
        assert!(effective_check_mode(false, true, false));
        assert!(effective_check_mode(true, false, false));
        assert!(!effective_check_mode(true, false, true));
    }
}
