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
    // A reader that stops reading — a `grep -q` that found its match, a
    // `head` that took what it wanted — closes the pipe. The message is
    // then incomplete, but the tool did what it was asked, so a broken
    // pipe is not an internal error: the exit code stays the one the
    // message asks for.
    let printed = error.print();
    let guard = if version {
        Some(writeln!(
            std::io::stdout().lock(),
            "guard: {} compiled deny entries",
            ruxel_guard::Policy::compiled_entries()
        ))
    } else {
        None
    };
    let broken = printed
        .as_ref()
        .err()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
        || guard.as_ref().is_some_and(|line| {
            line.as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
        });
    if (printed.is_err() || guard.as_ref().is_some_and(std::result::Result::is_err)) && !broken {
        ExitCode::Internal
    } else {
        code
    }
}
