use super::*;
use std::io::Cursor;

#[test]
fn bounded_reader_rejects_growth_and_preserves_binary_bytes() -> Result<()> {
    assert_eq!(
        read_bounded(Cursor::new([0xff, 0, 0x80]), 3)?,
        [0xff, 0, 0x80]
    );
    assert!(read_bounded(Cursor::new([1, 2, 3, 4]), 3).is_err());
    Ok(())
}

fn task(source: &str) -> v1::RenderedTask {
    v1::RenderedTask {
        module: "copy".into(),
        iterations: vec![v1::Iteration {
            params_json: serde_json::to_vec(&serde_json::json!({"src":source,"dest":"unused"}))
                .expect("fixture JSON"),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn sparse_oversize_rejects_and_binary_source_round_trips() -> Result<()> {
    let directory = Directory::new()?;
    File::create(directory.path().join("large"))?.set_len(MAX_SOURCE_BYTES + 1)?;
    let mut oversized = task("large");
    let error = prepare(&mut oversized, directory.path()).expect_err("oversized source");
    assert!(error.to_string().contains("32 MiB"));
    assert!(oversized.iterations[0].source_content.is_none());
    std::fs::write(directory.path().join("small"), [0xff, 0, 0x80])?;
    let mut small = task("small");
    prepare(&mut small, directory.path())?;
    assert_eq!(
        small.iterations[0].source_content.as_deref(),
        Some([0xff, 0, 0x80].as_slice())
    );
    let envelope = plan(small)?;
    let encoded = ruxel_proto::frame::encode_frame(&envelope);
    let decoded: v1::Envelope =
        ruxel_proto::frame::read_frame(&mut encoded.as_slice())?.context("decoded plan")?;
    assert_eq!(decoded, envelope);
    Ok(())
}

#[test]
fn aggregate_sources_and_complete_frame_are_bounded() -> Result<()> {
    let directory = Directory::new()?;
    File::create(directory.path().join("large"))?.set_len(MAX_SOURCE_BYTES)?;
    std::fs::write(directory.path().join("extra"), [1])?;
    let mut combined = task("large");
    prepare(&mut combined, directory.path())?;
    plan(combined.clone())?;
    combined = task("large");
    combined.iterations.extend(task("extra").iterations);
    assert!(prepare(&mut combined, directory.path()).is_err());
    let mut oversized = task("unused");
    oversized.name = "x".repeat(usize::try_from(ruxel_proto::frame::MAX_FRAME_LEN)?);
    assert!(plan(oversized).is_err());
    Ok(())
}

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ruxel-copy-bounds-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

#[test]
fn empty_sources_and_inline_content_keep_existing_semantics() -> Result<()> {
    let directory = Directory::new()?;
    File::create(directory.path().join("empty"))?;
    let mut empty = task("empty");
    prepare(&mut empty, directory.path())?;
    assert_eq!(empty.iterations[0].source_content, Some(Vec::new()));
    let mut inline = task("absent");
    inline.iterations[0].params_json = serde_json::to_vec(
        &serde_json::json!({"src":"absent","content":"inline","dest":"unused"}),
    )?;
    prepare(&mut inline, directory.path())?;
    assert!(inline.iterations[0].source_content.is_none());
    let params: Value = serde_json::from_slice(&inline.iterations[0].params_json)?;
    assert_eq!(params["content"], "inline");
    assert!(params.get("src").is_none());
    Ok(())
}
