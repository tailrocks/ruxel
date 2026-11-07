use anyhow::{Context, Result, ensure};
use std::{
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};
pub(super) const LIMIT: usize = 8 * 1024 * 1024;
pub(super) struct Capture {
    output: Receiver<io::Result<Vec<u8>>>,
    overflow: Arc<AtomicBool>,
}
impl Capture {
    pub(super) fn start(mut input: impl Read + Send + 'static) -> Self {
        let (sender, output) = mpsc::channel();
        let overflow = Arc::new(AtomicBool::new(false));
        let exceeded = overflow.clone();
        thread::spawn(move || {
            let mut read = || -> io::Result<Vec<u8>> {
                let mut bytes = Vec::new();
                let mut chunk = [0; 8192];
                loop {
                    let count = input.read(&mut chunk)?;
                    if count == 0 {
                        return Ok(bytes);
                    }
                    let retain = count.min(LIMIT - bytes.len());
                    bytes.extend_from_slice(&chunk[..retain]);
                    if retain < count {
                        exceeded.store(true, Ordering::Relaxed);
                    }
                }
            };
            let _ = sender.send(read());
        });
        Self { output, overflow }
    }
    pub(super) fn exceeded(&self) -> bool {
        self.overflow.load(Ordering::Relaxed)
    }
    pub(super) fn finish(self) -> Result<String> {
        let bytes = self
            .output
            .recv_timeout(Duration::from_secs(5))
            .context("capture pipe did not close after cleanup")??;
        ensure!(!self.exceeded(), "captured output exceeded {LIMIT} bytes");
        String::from_utf8(bytes).context("captured output is not UTF-8")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limit_accepts_exact_bytes_and_rejects_one_more() -> Result<()> {
        let data = vec![b'x'; LIMIT];
        assert_eq!(Capture::start(io::Cursor::new(data)).finish()?.len(), LIMIT);
        assert!(
            Capture::start(io::repeat(b'x').take((LIMIT + 1) as u64))
                .finish()
                .is_err()
        );
        Ok(())
    }
}
