pub(super) fn validate_privs(privs: &str, object_type: &str) -> Result<(), String> {
    let allowed: &[&str] = match object_type {
        "database" => &["CREATE", "CONNECT", "TEMPORARY", "TEMP"],
        "schema" => &["CREATE", "USAGE"],
        "table" | "default_privs" => &[
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
            "MAINTAIN",
        ],
        _ => {
            return Err(format!(
                "postgresql_privs: type {object_type:?} outside the closed surface"
            ));
        }
    };
    for p in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let upper = p.to_uppercase();
        if !matches!(upper.as_str(), "ALL" | "ALL PRIVILEGES") && !allowed.contains(&upper.as_str())
        {
            return Err(format!(
                "postgresql_privs: privilege {p:?} is invalid for type {object_type:?}"
            ));
        }
    }
    Ok(())
}

pub(super) fn acl_privileges(privs: &str, object_type: &str, server_version: u32) -> Vec<String> {
    let all: &[&str] = match object_type {
        "database" => &["CREATE", "CONNECT", "TEMPORARY"],
        "schema" => &["CREATE", "USAGE"],
        "table" | "default_privs" => &[
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
        ],
        _ => &[],
    };
    let mut result = Vec::new();
    for privilege in privs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let privilege = privilege.to_uppercase();
        if privilege == "ALL" || privilege == "ALL PRIVILEGES" {
            result.extend(all.iter().map(|p| (*p).to_owned()));
            if matches!(object_type, "table" | "default_privs") && server_version >= 170_000 {
                result.push("MAINTAIN".to_owned());
            }
        } else if privilege == "TEMP" {
            result.push("TEMPORARY".to_owned());
        } else {
            result.push(privilege);
        }
    }
    result.sort_unstable();
    result.dedup();
    result
}

pub(super) fn validate_role_attr_flags(flags: &str) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "SUPERUSER",
        "NOSUPERUSER",
        "CREATEDB",
        "NOCREATEDB",
        "CREATEROLE",
        "NOCREATEROLE",
        "INHERIT",
        "NOINHERIT",
        "LOGIN",
        "NOLOGIN",
        "REPLICATION",
        "NOREPLICATION",
        "BYPASSRLS",
        "NOBYPASSRLS",
    ];
    let tokens = role_flag_tokens(flags);
    for flag in &tokens {
        if !ALLOWED.contains(&flag.as_str()) {
            return Err(format!(
                "postgresql_user: role attribute {flag:?} outside the closed surface"
            ));
        }
        let opposite = flag
            .strip_prefix("NO")
            .map_or_else(|| format!("NO{flag}"), str::to_owned);
        if tokens.contains(&opposite) {
            return Err(format!(
                "postgresql_user: conflicting role attributes {flag} and {opposite}"
            ));
        }
    }
    Ok(())
}

pub(super) fn role_flag_tokens(flags: &str) -> Vec<String> {
    flags
        .split([',', ' '])
        .filter(|token| !token.is_empty())
        .map(str::to_uppercase)
        .collect()
}

pub(super) fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub(super) fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
