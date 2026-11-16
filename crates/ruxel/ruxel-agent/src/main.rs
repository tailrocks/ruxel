mod facts;
mod frames;
mod ledger;
mod modules;
mod sensitivity;
mod system_state;

use frames::{effective_check_mode, log_event, send_result};
use ruxel_proto::PROTO_VERSION;
use ruxel_proto::constants::HANDSHAKE_TIMEOUT_SECS;
use ruxel_proto::frame::{read_frame, write_frame};
use ruxel_proto::v1::{self, envelope::Msg};
use std::io::Write as _;
use std::sync::atomic::Ordering;

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
            std::thread::sleep(std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS));
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
    let mut plan_sysctl: Vec<(String, String, String)> = Vec::new();

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
        let is_hello = matches!(&envelope.msg, Some(Msg::Hello(_)));
        if is_hello == handshaken.load(Ordering::Relaxed) {
            log_event(
                &mut stdout,
                v1::log::Level::Error,
                if is_hello {
                    "duplicate Hello"
                } else {
                    "expected Hello before other messages"
                }
                .to_owned(),
            );
            ledger.flush();
            return 64;
        }
        match envelope.msg {
            Some(Msg::Hello(hello)) => {
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
                handshaken.store(true, Ordering::Relaxed);
                // The session's plans are one workflow's (the engine
                // frames every task as its own Plan): state begins here.
                ledger.begin_plan();
                modules::begin_plan();
                let ack = v1::Event {
                    msg: Some(v1::event::Msg::HelloAck(v1::HelloAck {
                        agent_version: env!("CARGO_PKG_VERSION").into(),
                        proto_version: PROTO_VERSION,
                        facts: Some(facts::gather()),
                        ledger_generation: 0,
                        sensitive_effects: Some(sensitive_effects.is_marked()),
                    })),
                };
                if write_frame(&mut stdout, &ack).is_err() {
                    return 64;
                }
            }
            Some(Msg::Done(done)) => {
                // Only a complete unfiltered plan expires unencountered records.
                if !done.filtered {
                    ledger.complete_plan();
                }
                ledger.flush();
                return 0;
            }
            Some(Msg::Plan(v1::Plan { tasks, .. }) | Msg::PlanPatch(v1::PlanPatch { tasks })) => {
                // Every frame is one task of the session's workflow; the
                // plan state began at Hello and every frame keeps it.
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
                        &mut plan_sysctl,
                    ) {
                        // The plan halted before its last task: the tasks it never reached are
                        // still the workflow's, so their records must not expire at the plan's end.
                        ledger.abort_plan();
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
    plan_sysctl: &mut Vec<(String, String, String)>,
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
        // The plan has encountered this task whatever the run does with it — cache
        // hit, check-mode run, unrecorded run — and its end expires only records it never saw.
        ledger.encounter(&iteration.ledger_key);
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

        // The plan's sysctl entries in delivery order, not the ledger's dropped-task history.
        if let Some(declared) = modules::sysctl_plan_declaration(&task.module, &params)
            && !plan_sysctl.contains(&declared)
        {
            plan_sysctl.push(declared);
        }

        // The preflight's managed set, shared with the cache below.
        let managed = ledger.sysctl_preflight_managed(&task.module, plan_sysctl);
        let key = &iteration.ledger_key;
        if !no_cache
            && task.become_user.is_empty()
            && ledger::cache_allowed(&task.module, &params)
            && !task_check_mode
            && !task.no_log
            && !iteration.ledger_key.is_empty()
            // The plan's own earlier steps can renumber an identity the
            // task names: `usermod -u` leaves an arbitrary file at the
            // old number, and the cached probe weighs the file's number
            // alone — the file would keep the number the plan gave out.
            && !modules::ownership_cache_stale(&task.module, &params)
            && let Some(cached) = ledger.cached_ok_with_managed(key, system_state, &managed)
        {
            // A forced task the ledger satisfies still observes the host,
            // reconciling its own predictions so no later task judges a stale one.
            if task.module == "systemd" {
                modules::systemd::reconcile_predictions(&params, system_state);
            }
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
            &managed,
        );
        if !task_check_mode {
            if task.no_log {
                // The result and its probes stay out of the ledger;
                // the sysctl declaration is the workflow's own.
                ledger.record_managed_declaration(
                    &iteration.ledger_key,
                    &task.module,
                    &params,
                    outcome.status,
                );
            } else {
                ledger.record_with_state(
                    &iteration.ledger_key,
                    &task.module,
                    &params,
                    outcome.status,
                    &outcome.result,
                    system_state,
                );
            }
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
