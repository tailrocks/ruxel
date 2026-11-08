use anyhow::{Context, Result, ensure};
use prost::Message;
use ruxel_proto::v1;
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path},
};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

pub(super) fn prepare(task: &mut v1::RenderedTask, directory: &Path) -> Result<()> {
    if task.module != "copy" {
        return Ok(());
    }
    let mut remaining = MAX_SOURCE_BYTES;
    for iteration in &mut task.iterations {
        let mut params: Map<String, Value> = serde_json::from_slice(&iteration.params_json)?;
        let source = params.remove("src").filter(|value| !value.is_null());
        if let Some(source) = source.filter(|_| params.get("content").is_none_or(Value::is_null)) {
            let file = open_source(
                directory,
                source.as_str().context("source path must be text")?,
            )?;
            let metadata = file.metadata().context("inspect controller copy source")?;
            ensure!(
                metadata.is_file(),
                "controller copy source must be a regular file"
            );
            ensure!(
                metadata.len() <= remaining,
                "controller copy sources exceed supported aggregate limit of 32 MiB per task request"
            );
            let bytes = read_bounded(file, remaining)?;
            remaining -= u64::try_from(bytes.len())?;
            iteration.source_content = Some(bytes);
            params.remove("content");
        }
        iteration.params_json = serde_json::to_vec(&params)?;
    }
    Ok(())
}

fn open_source(directory: &Path, source: &str) -> Result<File> {
    use rustix::fs::{Mode, OFlags, open, openat};
    let components = Path::new(source).components().collect::<Vec<_>>();
    ensure!(
        !components.is_empty()
            && components
                .iter()
                .all(|part| matches!(part, Component::Normal(_))),
        "controller copy source must be a relative path without traversal"
    );
    let mut parent = open(
        directory,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .context("open controller copy source directory")?;
    for (index, part) in components.iter().enumerate() {
        let Component::Normal(name) = part else {
            unreachable!()
        };
        let last = index + 1 == components.len();
        let mut flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        if !last {
            flags |= OFlags::DIRECTORY;
        }
        let next =
            openat(&parent, *name, flags, Mode::empty()).context("open controller copy source")?;
        if last {
            return Ok(File::from(next));
        }
        parent = next;
    }
    unreachable!()
}

fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .context("read controller copy source")?;
    ensure!(
        u64::try_from(bytes.len())? <= limit,
        "controller copy sources exceed supported aggregate limit of 32 MiB per task request"
    );
    Ok(bytes)
}

pub(super) fn plan(task: v1::RenderedTask) -> Result<v1::Envelope> {
    let envelope = v1::Envelope {
        msg: Some(v1::envelope::Msg::Plan(v1::Plan {
            tasks: vec![task],
            blobs_referenced: vec![],
        })),
    };
    ensure!(
        u64::try_from(envelope.encoded_len())? <= ruxel_proto::frame::MAX_FRAME_LEN,
        "task plan exceeds supported protocol frame limit of 64 MiB"
    );
    Ok(envelope)
}

#[cfg(test)]
mod tests;
