//! Ruxel agent: runs on the target host, executes compiled plans streamed
//! over stdio (varint-framed protobuf — docs/ARCHITECTURE.md §2). M2
//! skeleton: handshake + facts + clean shutdown + crash reporting + the
//! single-run lock. Module runtime, probe engine, and ledger land in M3.
//!
//! stdout is exclusively protocol frames; anything human-readable goes out
//! as Log events or to stderr.

mod facts;
mod modules;

use ruxel_proto::PROTO_VERSION;
use ruxel_proto::frame::{read_frame, write_frame};
use ruxel_proto::v1::{self, envelope::Msg};
use std::io::Write as _;

/// Exit codes: 0 clean, 64 protocol error, 65 version mismatch,
/// 66 lock held (another run in flight), 70 panic.
fn main() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
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

fn state_dir() -> std::path::PathBuf {
    std::env::var_os("RUXEL_STATE_DIR")
        .map(Into::into)
        .unwrap_or_else(|| "/var/lib/ruxel".into())
}

fn serve() -> i32 {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    // Single-run guard (ARCHITECTURE §8): one agent per host at a time.
    let dir = state_dir();
    let _ = std::fs::create_dir_all(&dir);
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

    let mut check_mode = false;

    loop {
        let envelope: v1::Envelope = match read_frame(&mut stdin) {
            Ok(Some(env)) => env,
            // Clean EOF: controller went away (Ctrl-C, connection loss).
            // The current task always completes before the next frame read,
            // so exiting here leaves the host reusable (ARCHITECTURE §8).
            Ok(None) => return 0,
            Err(e) => {
                log_event(&mut stdout, v1::log::Level::Error, format!("frame: {e}"));
                return 64;
            }
        };
        match envelope.msg {
            Some(Msg::Hello(hello)) => {
                check_mode = hello.check_mode;
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
                    })),
                };
                if write_frame(&mut stdout, &ack).is_err() {
                    return 64;
                }
            }
            Some(Msg::Done(_)) => {
                // Ledger flush goes here when the ledger lands.
                return 0;
            }
            Some(Msg::Plan(v1::Plan { tasks, .. }))
            | Some(Msg::PlanPatch(v1::PlanPatch { tasks })) => {
                for task in &tasks {
                    execute_task(&mut stdout, task, check_mode);
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

/// Execute one rendered task: per iteration, TaskStart then TaskResult.
/// Aggregation, register binding, and status envelopes are controller-side
/// (task_eval) — the agent reports raw per-iteration module outcomes.
fn execute_task(out: &mut impl std::io::Write, task: &v1::RenderedTask, check_mode: bool) {
    let task_check_mode = check_mode && !task.check_mode_override;
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

        let params: serde_json::Value = if iteration.params_json.is_empty() {
            serde_json::json!({})
        } else {
            match serde_json::from_slice(&iteration.params_json) {
                Ok(v) => v,
                Err(e) => {
                    send_result(
                        out,
                        task.task_id,
                        iteration,
                        "failed",
                        true,
                        &serde_json::json!({"failed": true, "msg": format!("bad params: {e}")}),
                        start,
                    );
                    continue;
                }
            }
        };

        // check-mode skip for command/shell (SEMANTICS §3.5) — predicted
        // as "skipped" by the agent so timing stays honest.
        if task_check_mode && matches!(task.module.as_str(), "command" | "shell") {
            send_result(
                out,
                task.task_id,
                iteration,
                "skipped",
                false,
                &serde_json::json!({
                    "changed": false,
                    "skipped": true,
                    "msg": "remote module (command/shell) does not support check mode",
                }),
                start,
            );
            continue;
        }

        let ctx = modules::ExecContext {
            check_mode: task_check_mode,
            environment: task
                .environment
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        let outcome = modules::execute(&task.module, &params, &iteration.free_form, &ctx);
        send_result(
            out,
            task.task_id,
            iteration,
            outcome.status,
            outcome.changed,
            &outcome.result,
            start,
        );
    }
}

fn send_result(
    out: &mut impl std::io::Write,
    task_id: u64,
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
                task_id,
                status: status.to_string(),
                changed,
                result_json: serde_json::to_vec(result).unwrap_or_default(),
                diff: String::new(),
                elapsed_ms: start.elapsed().as_millis() as u64,
                item_label: iteration.item_label.clone(),
            })),
        },
    );
}

fn log_event(out: &mut impl std::io::Write, level: v1::log::Level, message: String) {
    let event = v1::Event {
        msg: Some(v1::event::Msg::Log(v1::Log {
            level: level as i32,
            message,
        })),
    };
    let _ = write_frame(out, &event);
}
