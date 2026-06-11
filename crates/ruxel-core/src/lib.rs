//! Ruxel controller core: the typed model of the workload's playbooks and
//! inventory, the parser that enforces the closed surface
//! (docs/SEMANTICS.md), the MiniJinja templating engine, and the plan
//! compiler. Filled in across M1 (docs/PLAN.md).

pub mod engine;
pub mod inventory;
pub mod modules;
pub mod playbook;
pub mod task_eval;
