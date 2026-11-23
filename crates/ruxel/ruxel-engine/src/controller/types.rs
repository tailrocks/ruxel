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
    /// The plan's static sysctl declarations, whose unseen entries ride
    /// each frame so the agent weighs later stages' files before they
    /// execute; entries the session already received stay behind.
    pub plan_sysctl: Vec<super::v1::SysctlDeclaration>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recap {
    // `delegated` rides beside the counts: every session this origin
    // sent a request to, so an incomplete origin marks its destinations
    // incomplete too instead of leaving them an unrelated clean recap.
    pub delegated: std::collections::BTreeSet<String>,
    pub ok: u64,
    pub changed: u64,
    pub failed: u64,
    /// Connection failures included in `failed`, distinct from task failures.
    pub unreachable: u64,
    pub skipped: u64,
    pub rescued: u64,
    pub ignored: u64,
    pub planned: u64,
    /// Selection omitted tasks the workflow keeps: at least one leaf
    /// was skipped for its tags or its `when` condition, so the run
    /// is a filtered subset and its end must expire nothing.
    pub filtered: bool,
    /// The session stopped before its tasks were all sent: evaluation
    /// or rendering failed mid-play, or the host failed and its later
    /// plays never ran. The agent never receives those tasks and never
    /// calls `abort_plan`, so the session's end is no verdict on their
    /// records and must expire nothing — the same no-prune semantics as
    /// a filtered run.
    pub aborted: bool,
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
