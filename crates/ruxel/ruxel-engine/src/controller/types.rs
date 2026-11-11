#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plan,
    Apply,
    Check,
    Diff,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub host: String,
    pub task: super::v1::RenderedTask,
    /// The keyed fingerprint of the task's secret-bearing content, built
    /// from the selectors alone before the content was evaluated.
    pub content_fingerprint: Option<String>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recap {
    pub ok: u64,
    pub changed: u64,
    pub failed: u64,
    /// Connection failures included in `failed`, distinct from task failures.
    pub unreachable: u64,
    pub skipped: u64,
    pub rescued: u64,
    pub ignored: u64,
    pub planned: u64,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Apply,
            hosts: Vec::new(),
            tags: Vec::new(),
        }
    }
}
