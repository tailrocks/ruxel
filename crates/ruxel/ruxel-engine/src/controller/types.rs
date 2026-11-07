/// Execution mode selected by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plan,
    Apply,
    Check,
    Diff,
}

/// An operation prepared for an agent, with its intended execution host.
#[derive(Debug, Clone)]
pub struct Request {
    pub host: String,
    pub task: super::v1::RenderedTask,
}
