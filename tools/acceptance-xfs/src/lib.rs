#[cfg(test)]
#[allow(dead_code)]
fn xfs_mount(_: &std::fs::File) -> Result<std::fs::File, String> {
    unreachable!("synthetic tests do not discover mounts")
}

#[cfg(test)]
#[allow(dead_code)]
fn run_on_file(_: &std::fs::File, _: &str, _: &[&str]) -> Result<std::process::Output, String> {
    unreachable!("synthetic tests do not query block devices")
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../crates/ruxel/ruxel-agent/src/modules/filesystem/xfs.rs"]
mod xfs;
