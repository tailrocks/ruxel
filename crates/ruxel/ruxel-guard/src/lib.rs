#[cfg(test)]
mod cidr_file;
mod mux;
mod policy;
mod resolve;
mod scan;
mod target;

pub use policy::{Policy, PolicyError};
pub use scan::deny_hits;
pub use target::{Guard, GuardedTarget, HostSpec, Refusal, Refusals};
include!(concat!(env!("OUT_DIR"), "/compiled_deny.rs"));

#[cfg(test)]
mod tests;
