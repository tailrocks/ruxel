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
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recap {
    pub ok: u64,
    pub changed: u64,
    pub failed: u64,
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
