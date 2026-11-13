//! The host's account and group entries: the passwd and group content
//! every account and group operation reads before it touches anything.

#[derive(Default)]
pub(super) struct PasswdEntry {
    pub(super) uid: u64,
    pub(super) gid: u64,
    pub(super) comment: String,
    pub(super) home: String,
    pub(super) shell: String,
}

pub(super) fn group_entry(name: &str) -> Result<Option<(String, u64)>, String> {
    let content = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    group_entry_from(&content, name)
}

pub(super) fn group_entry_from(content: &str, name: &str) -> Result<Option<(String, u64)>, String> {
    for line in content.lines() {
        let mut f = line.split(':');
        if f.next() == Some(name) {
            let gid = f
                .nth(1)
                .and_then(|g| g.parse().ok())
                .ok_or("malformed group line")?;
            return Ok(Some((name.to_string(), gid)));
        }
    }
    Ok(None)
}

pub(super) fn passwd_entry(name: &str) -> Result<Option<PasswdEntry>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    passwd_entry_from(&passwd, name)
}

pub(super) fn passwd_entry_from(passwd: &str, name: &str) -> Result<Option<PasswdEntry>, String> {
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() == Some(&name) && f.len() >= 7 {
            return Ok(Some(PasswdEntry {
                uid: f[2].parse().map_err(|_| "bad uid")?,
                gid: f[3].parse().map_err(|_| "bad gid")?,
                comment: f[4].to_string(),
                home: f[5].to_string(),
                shell: f[6].to_string(),
            }));
        }
    }
    Ok(None)
}

/// The account that holds `uid` on the host: the passwd database's
/// entry with that number, apart from `name` itself — the account's
/// own number is no collision with itself. `useradd -u` and
/// `usermod -u` refuse a number another account holds, so the request
/// is checked against the live database before anything runs.
pub(super) fn uid_holder(uid: u64, name: &str) -> Result<Option<String>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    Ok(uid_holder_from(&passwd, uid, name))
}

pub(super) fn uid_holder_from(passwd: &str, uid: u64, name: &str) -> Option<String> {
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(uid) && f.first() != Some(&name) {
            return Some(f[0].to_string());
        }
    }
    None
}

/// The group that holds `gid` on the host: the group database's entry
/// with that number, apart from `name` itself. `groupadd -g` and
/// `groupmod -g` refuse a number another group holds.
pub(super) fn gid_holder(gid: u64, name: &str) -> Result<Option<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    Ok(gid_holder_from(&groups, gid, name))
}

pub(super) fn gid_holder_from(groups: &str, gid: u64, name: &str) -> Option<String> {
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[2].parse::<u64>() == Ok(gid) && f.first() != Some(&name) {
            return Some(f[0].to_string());
        }
    }
    None
}

pub(super) fn supplementary_groups(name: &str) -> Result<Vec<String>, String> {
    let groups = std::fs::read_to_string("/etc/group").map_err(|e| e.to_string())?;
    Ok(supplementary_groups_from(&groups, name))
}

pub(super) fn supplementary_groups_from(groups: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in groups.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[3].split(',').any(|m| m == name) {
            out.push(f[0].to_string());
        }
    }
    out
}

pub(super) fn groups_need_update(current: &[String], want: &[String], append: bool) -> bool {
    let missing = want.iter().any(|group| !current.contains(group));
    (append && missing)
        || (!append
            && (current.len() != want.len() || want.iter().any(|group| !current.contains(group))))
}
