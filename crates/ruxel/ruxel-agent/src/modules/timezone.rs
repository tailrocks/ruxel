use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("timezone: name required")?;
    let current = std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|path| timezone_from_link(&path))
        .unwrap_or_default();
    let changed = current != name;
    if changed && !ctx.check_mode {
        let mut command = std::process::Command::new("timedatectl");
        command.args(["set-timezone", name]);
        super::run_checked(command)?;
    }
    Ok(json!({"changed": changed, "failed": false}))
}

fn timezone_from_link(path: &std::path::Path) -> Option<String> {
    path.to_string_lossy()
        .split("/zoneinfo/")
        .nth(1)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_timezone_link() {
        assert_eq!(
            super::timezone_from_link(std::path::Path::new("/usr/share/zoneinfo/Asia/Tokyo"))
                .as_deref(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            super::timezone_from_link(std::path::Path::new("/etc/localtime")),
            None
        );
    }
}
