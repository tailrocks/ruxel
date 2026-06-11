//! Ruxel agent: runs on the target host, executes compiled plans streamed
//! over stdio (varint-framed protobuf — docs/ARCHITECTURE.md §2). M2
//! skeleton: handshake + facts + clean shutdown + crash reporting + the
//! single-run lock. Module runtime, probe engine, and ledger land in M3.
//!
//! stdout is exclusively protocol frames; anything human-readable goes out
//! as Log events or to stderr.

mod facts;

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

    loop {
        let envelope: v1::Envelope = match read_frame(&mut stdin) {
            Ok(Some(env)) => env,
            // Clean EOF: controller went away (Ctrl-C, connection loss).
            // Nothing in flight in M2 — exit reusable (ARCHITECTURE §8).
            Ok(None) => return 0,
            Err(e) => {
                log_event(&mut stdout, v1::log::Level::Error, format!("frame: {e}"));
                return 64;
            }
        };
        match envelope.msg {
            Some(Msg::Hello(hello)) => {
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
                // Ledger flush goes here in M3.
                return 0;
            }
            Some(Msg::Plan(_)) | Some(Msg::PlanPatch(_)) | Some(Msg::Resume(_)) => {
                log_event(
                    &mut stdout,
                    v1::log::Level::Warn,
                    "plan execution lands in M3; ignoring".to_string(),
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

fn log_event(out: &mut impl std::io::Write, level: v1::log::Level, message: String) {
    let event = v1::Event {
        msg: Some(v1::event::Msg::Log(v1::Log {
            level: level as i32,
            message,
        })),
    };
    let _ = write_frame(out, &event);
}
