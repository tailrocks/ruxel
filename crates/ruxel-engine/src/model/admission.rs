use super::validate_config;
use anyhow::{Context, Result};
use serde_json::Value;

/// Validated configuration and the complete evaluated input for later admission.
/// Construction performs no connections or secret lookups.
pub struct AdmissionConfig {
    evaluated: Value,
    normalized: Value,
    ssh: ruxel_config::model::Ssh,
    guard: ruxel_config::model::Guard,
}

impl std::fmt::Debug for AdmissionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionConfig").finish_non_exhaustive()
    }
}

impl AdmissionConfig {
    /// Validate the canonical config and the separately evaluated guard object.
    ///
    /// # Errors
    /// Rejects malformed configuration, missing SSH settings, or invalid guard policy.
    pub fn new(evaluated: &Value, guard: &Value) -> Result<Self> {
        let normalized = validate_config(evaluated)?;
        let ssh = serde_json::from_value(
            normalized
                .get("ssh")
                .cloned()
                .context("missing SSH configuration")?,
        )
        .context("invalid SSH configuration")?;
        let guard = serde_json::from_value(guard.clone()).context("invalid guard configuration")?;
        Ok(Self {
            evaluated: evaluated.clone(),
            normalized,
            ssh,
            guard,
        })
    }

    #[must_use]
    pub fn evaluated(&self) -> &Value {
        &self.evaluated
    }
    #[must_use]
    pub fn normalized(&self) -> &Value {
        &self.normalized
    }
    #[must_use]
    pub fn ssh(&self) -> &ruxel_config::model::Ssh {
        &self.ssh
    }
    #[must_use]
    pub fn guard(&self) -> &ruxel_config::model::Guard {
        &self.guard
    }
}
