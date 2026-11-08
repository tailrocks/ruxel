use super::Execution;
use crate::controller::Backend;
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(super) struct SourceRoot {
    path: PathBuf,
    directory: File,
}

impl SourceRoot {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let path = std::path::absolute(path)?;
        let directory = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )
        .context("open template source directory")?;
        Ok(Self {
            path,
            directory: directory.into(),
        })
    }
}

pub(super) fn prepare(
    execution: &mut Execution,
    kind: &str,
    params: &mut Map<String, Value>,
    scope: &Map<String, Value>,
) -> Result<()> {
    if kind != "template" || params.get("content").is_some_and(|v| !v.is_null()) {
        return Ok(());
    }
    let source = params
        .get("src")
        .context("template requires src or content")?
        .clone();
    let source = execution.evaluate(&source, scope)?;
    let root = &execution.template_root.path;
    let source = Path::new(
        source
            .as_str()
            .context("template source path must be text")?,
    );
    let relative = if source.is_absolute() {
        source
            .strip_prefix(root)
            .context("template source must be inside source directory")?
    } else {
        source
    };
    let components = relative.components().collect::<Vec<_>>();
    ensure!(
        !components.is_empty()
            && components
                .iter()
                .all(|part| matches!(part, Component::Normal(_))),
        "template source must be a path without traversal"
    );
    let path = root.join(relative);
    ensure!(
        path.extension().is_some_and(|extension| extension == "pkl"),
        "template source must be a Pkl module"
    );
    let mut parent = execution.template_root.directory.try_clone()?;
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            unreachable!()
        };
        let last = index + 1 == components.len();
        let mut flags = rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW;
        if !last {
            flags |= rustix::fs::OFlags::DIRECTORY;
        }
        let next = rustix::fs::openat(&parent, *name, flags, rustix::fs::Mode::empty())
            .context("open template source")?;
        if last {
            let mut file = File::from(next);
            ensure!(
                file.metadata()?.is_file(),
                "template source must be a regular file"
            );
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .context("read template source")?;
            let expression = ruxel_config::eval_source(&contents, &path)
                .context("evaluate typed template source")?;
            crate::model::validate_template(&expression)?;
            params.remove("src");
            params.insert("content".into(), expression);
            return Ok(());
        }
        parent = next.into();
    }
    unreachable!()
}
