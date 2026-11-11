//! The secret references a workflow names: what store, which selector.

use std::fmt;

#[derive(Clone, Eq, Hash, PartialEq)]
pub enum Lookup {
    Fnox {
        key: String,
    },
    /// A generated key reads like `Fnox`; at apply time the run additionally
    /// creates it, once, under the fnox write lock, when it is missing.
    /// `plan`, `check` and `diff` never generate: they run in synthetic mode,
    /// which contacts no store and answers with a selector-derived
    /// placeholder. Synthetic mode is rotation-blind by design: the
    /// placeholder derives from the selector, not the value, so no plan or
    /// diff ever detects a rotated value.
    FnoxGenerated {
        key: String,
        format: String,
    },
    OnePassword {
        vault: String,
        item: String,
        field: String,
        section: Option<String>,
    },
    OnePasswordUri {
        reference: String,
    },
    Command {
        argv: Vec<String>,
    },
    Synthetic {
        value: String,
    },
}

impl fmt::Debug for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fnox { .. } | Self::FnoxGenerated { .. } => "Fnox([REDACTED])",
            Self::OnePassword { .. } | Self::OnePasswordUri { .. } => "OnePassword([REDACTED])",
            Self::Command { .. } => "Command([REDACTED])",
            Self::Synthetic { .. } => "Synthetic([REDACTED])",
        })
    }
}
