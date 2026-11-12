//! The task environment reaches every debconf command: the
//! `DEBCONF_DB_OVERRIDE` and `DEBCONF_DB_FALLBACK` values a rendered task
//! carries steer which database the tools touch, so a command that
//! ignored the environment would silently answer and write the default
//! one.

use super::*;
use std::os::unix::fs::PermissionsExt as _;

/// The selections reader runs under the task environment: a fixture
/// reports the value it sees as the answer of the question.
#[test]
fn the_task_environment_reaches_the_selections_reader() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("ruxel-debconf-env-{}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let reader = directory.join("reader");
    std::fs::write(
        &reader,
        "#!/bin/sh\nprintf 'nginx\\tnginx/nginx-user\\tstring\\t%s\\n' \"${FIXTURE:-absent}\"\n",
    )
    .map_err(|error| error.to_string())?;
    std::fs::set_permissions(&reader, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let mut ctx = context(false);
    ctx.environment.push(("FIXTURE".into(), "present".into()));
    let Ok(text) = current_selections_named(reader.to_str().unwrap_or_default(), &ctx) else {
        return Err("the reader did not answer".into());
    };
    let answer = answer_of(&text, "nginx", "nginx/nginx-user");
    let seen = answer.map(|answer| answer.value).unwrap_or_default();
    std::fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
    assert_eq!(seen, "present", "the reader must see the task environment");
    Ok(())
}

/// The selection writer runs under the task environment: a fixture logs
/// the value it sees.
#[test]
fn the_task_environment_reaches_the_selection_writer() -> Result<(), String> {
    let directory =
        std::env::temp_dir().join(format!("ruxel-debconf-env-set-{}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let log = directory.join("seen");
    let writer = directory.join("writer");
    std::fs::write(
        &writer,
        format!(
            "#!/bin/sh\n/bin/cat > /dev/null\nprintf '%s\\n' \"${{FIXTURE:-absent}}\" >> {}\n",
            log.display()
        ),
    )
    .map_err(|error| error.to_string())?;
    std::fs::set_permissions(&writer, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let mut ctx = context(false);
    ctx.environment.push(("FIXTURE".into(), "present".into()));
    set_selection_named(
        writer.to_str().unwrap_or_default(),
        "nginx nginx/nginx-user string www-data\n",
        &ctx,
    )?;
    let seen = std::fs::read_to_string(&log).map_err(|error| error.to_string())?;
    std::fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
    assert_eq!(
        seen, "present\n",
        "the writer must see the task environment"
    );
    Ok(())
}
