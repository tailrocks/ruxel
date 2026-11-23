//! The frames the agent writes back to the controller: task results and
//! log events, and the check-mode rule that decides a task's own mode.

use ruxel_proto::frame::write_frame;
use ruxel_proto::v1;

pub(crate) fn effective_check_mode(global: bool, force: bool, override_off: bool) -> bool {
    (global || force) && !override_off
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::let_underscore_must_use,
    reason = "replaced by the Ruxel rewrite"
)]
pub(crate) fn send_result(
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
pub(crate) fn log_event(out: &mut impl std::io::Write, level: v1::log::Level, message: String) {
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
