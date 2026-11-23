use super::Recap;
use std::collections::BTreeMap;

pub(crate) struct RunFailure {
    pub(crate) recaps: BTreeMap<String, Recap>,
    source: anyhow::Error,
}
impl RunFailure {
    pub(crate) fn new(source: anyhow::Error, recaps: BTreeMap<String, Recap>) -> Self {
        Self { recaps, source }
    }
}
impl std::fmt::Debug for RunFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunFailure")
            .field("recaps", &self.recaps)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for RunFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, formatter)
    }
}
impl std::error::Error for RunFailure {}
