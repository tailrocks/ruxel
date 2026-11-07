#[derive(Debug, Clone, Copy)]
pub enum ExitCode {
    Success = 0,
    Internal = 1,
    Usage = 2,
    Changes = 3,
    Config = 4,
    GuardRefused = 5,
    Secrets = 6,
    Unreachable = 7,
    Failed = 8,
    Interrupted = 130,
}
impl ExitCode {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }
}
