use ruxel::exit::ExitCode;
use std::io::Write;

/// Print a clap usage, help, or version message. `--version` also reports
/// the number of deny entries compiled into the connection guard.
pub(crate) fn report(error: &clap::Error) -> ExitCode {
    let code = if error.use_stderr() {
        ExitCode::Usage
    } else {
        ExitCode::Success
    };
    let version = error.kind() == clap::error::ErrorKind::DisplayVersion;
    if error.print().is_err()
        || (version
            && writeln!(
                std::io::stdout().lock(),
                "guard: {} compiled deny entries",
                ruxel_guard::Policy::compiled_entries()
            )
            .is_err())
    {
        ExitCode::Internal
    } else {
        code
    }
}
