use super::*;
use std::{
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn fifo_without_writer_is_rejected_promptly() -> Result<()> {
    const CHILD_PATH: &str = "RUXEL_SYNTHETIC_FIFO_DIRECTORY";
    if let Some(directory) = std::env::var_os(CHILD_PATH) {
        let mut request = task("fifo");
        let error = prepare(&mut request, Path::new(&directory)).expect_err("FIFO rejected");
        assert!(error.to_string().contains("regular file"));
        return Ok(());
    }
    let directory = Directory::new()?;
    let status = Command::new("mkfifo")
        .arg(directory.path().join("fifo"))
        .status()?;
    assert!(status.success());
    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "runtime::copy_sources::tests::fifo::fifo_without_writer_is_rejected_promptly",
        ])
        .env(CHILD_PATH, directory.path())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait()? {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("FIFO source open blocked without a writer");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
