#![expect(
    clippy::expect_used,
    clippy::format_collect,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    clippy::unwrap_used,
    missing_debug_implementations,
    reason = "removed with the YAML front-end"
)]
#![cfg_attr(
    test,
    expect(
        clippy::let_underscore_must_use,
        clippy::many_single_char_names,
        reason = "removed with the YAML front-end"
    )
)]

pub mod compiler;
pub mod engine;
pub mod inventory;
pub mod modules;
pub mod playbook;
pub mod task_eval;
