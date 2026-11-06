//! Controller library surface: transport (SSH + agent lifecycle) and,
//! later, the scheduler/reporter. The `ruxel` binary in main.rs is the
//! CLI over this.

pub mod scheduler;
pub mod secrets;
pub mod transport;
