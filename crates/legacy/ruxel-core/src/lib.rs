//! Ruxel controller core: the typed model of the workload's playbooks and
//! inventory, the parser that enforces the closed surface
//! (docs/SEMANTICS.md), the `MiniJinja` templating engine, and the plan
//! compiler. Filled in across M1 (docs/PLAN.md).

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
