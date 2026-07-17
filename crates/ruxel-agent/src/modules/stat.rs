//! `stat` (SEMANTICS §6): read-only, never changed. Returns the `stat.*`
//! fields the workload consumes (exists, isblk, …) plus the cheap common
//! ones.

use super::{params_object, str_param};
use serde_json::{Value, json};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

pub fn run(params: &Value) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("stat: path required")?;
    let follow = super::bool_param(obj, "follow", false);

    let meta = if follow {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    };

    let stat = match meta {
        Err(_) => json!({"exists": false}),
        Ok(m) => {
            let ft = m.file_type();
            json!({
                "exists": true,
                "isdir": ft.is_dir(),
                "isreg": ft.is_file(),
                "islnk": ft.is_symlink(),
                "isblk": ft.is_block_device(),
                "ischr": ft.is_char_device(),
                "isfifo": ft.is_fifo(),
                "issock": ft.is_socket(),
                "mode": format!("0{:o}", m.permissions().mode() & 0o7777),
                "uid": m.uid(),
                "gid": m.gid(),
                "size": m.size(),
                "path": path,
            })
        }
    };
    Ok(json!({"stat": stat, "changed": false, "failed": false}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::os::unix::fs::symlink;

    #[test]
    fn maps_regular_missing_and_symlink_fields() {
        let root = std::env::temp_dir().join(format!("ruxel-stat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let file = root.join("file");
        let link = root.join("link");
        std::fs::write(&file, "abc").unwrap();
        symlink(&file, &link).unwrap();
        assert_eq!(
            super::run(&json!({"path": file})).unwrap()["stat"]["isreg"],
            true
        );
        assert_eq!(
            super::run(&json!({"path": link})).unwrap()["stat"]["islnk"],
            true
        );
        assert_eq!(
            super::run(&json!({"path": root.join("missing")})).unwrap()["stat"]["exists"],
            false
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
