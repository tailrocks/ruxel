//! The closed module surface (docs/SEMANTICS.md §6): exactly the modules and
//! parameters the workload uses. Unknown module, unknown parameter, or (for
//! literal values) a value outside the observed set is a hard parse error —
//! that is what "closed surface" means.

/// One module's allowed surface.
#[derive(Debug, Clone, Copy)]
pub struct ModuleSurface {
    pub name: &'static str,
    /// Allowed parameter keys.
    pub params: &'static [&'static str],
    /// Accepts a free-form string body (`shell: echo hi`).
    pub free_form: bool,
    /// Allowed literal values per param (templated values are validated
    /// after rendering, not at parse time). Empty = any value.
    pub literal_enums: &'static [(&'static str, &'static [&'static str])],
    /// Params accepted via the `args:` keyword (shell only in the workload).
    pub args_params: &'static [&'static str],
    /// `set_fact` semantics: params are arbitrary variable names.
    pub any_params: bool,
}

const fn surface(name: &'static str, params: &'static [&'static str]) -> ModuleSurface {
    ModuleSurface {
        name,
        params,
        free_form: false,
        literal_enums: &[],
        args_params: &[],
        any_params: false,
    }
}

/// The registry. Source of truth: the 2026-06-11 param/value extraction in
/// docs/SEMANTICS.md §6. Keep sorted by name for the error listing.
pub static MODULES: &[ModuleSurface] = &[
    ModuleSurface {
        literal_enums: &[("state", &["mounted"])],
        ..surface(
            "ansible.posix.mount",
            &["fstype", "opts", "path", "src", "state"],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface(
            "ansible.posix.sysctl",
            &["name", "reload", "state", "sysctl_set", "value"],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["present", "latest"])],
        ..surface(
            "apt",
            &[
                "autoremove",
                "force",
                "name",
                "state",
                "update_cache",
                "upgrade",
            ],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface(
            "apt_repository",
            &["filename", "repo", "state", "update_cache"],
        )
    },
    surface("assert", &["fail_msg", "that"]),
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface("authorized_key", &["key", "state", "user"])
    },
    surface(
        "blockinfile",
        &["block", "create", "group", "mode", "owner", "path"],
    ),
    ModuleSurface {
        free_form: true,
        ..surface("command", &["argv", "chdir", "cmd"])
    },
    surface("community.general.lvg", &["pvs", "vg"]),
    surface("community.general.lvol", &["lv", "resizefs", "size", "vg"]),
    surface("community.general.timezone", &["name"]),
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface(
            "community.postgresql.postgresql_db",
            &["login_port", "login_user", "name", "owner", "state"],
        )
    },
    ModuleSurface {
        literal_enums: &[
            ("state", &["present"]),
            ("type", &["database", "schema", "table", "default_privs"]),
        ],
        ..surface(
            "community.postgresql.postgresql_privs",
            &[
                "login_db",
                "login_port",
                "login_user",
                "objs",
                "privs",
                "role",
                "schema",
                "state",
                "type",
            ],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface(
            "community.postgresql.postgresql_schema",
            &["login_db", "login_port", "login_user", "name", "state"],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface(
            "community.postgresql.postgresql_user",
            &[
                "login_port",
                "login_user",
                "name",
                "password",
                "role_attr_flags",
                "state",
            ],
        )
    },
    surface(
        "copy",
        &["content", "dest", "force", "group", "mode", "owner", "src"],
    ),
    surface("debug", &["msg"]),
    surface("fail", &["msg"]),
    ModuleSurface {
        literal_enums: &[("state", &["directory", "absent", "link"])],
        ..surface(
            "file",
            &[
                "dest", "group", "mode", "owner", "path", "recurse", "src", "state",
            ],
        )
    },
    ModuleSurface {
        literal_enums: &[("fstype", &["xfs", "ext4"])],
        ..surface("filesystem", &["dev", "fstype", "resizefs"])
    },
    surface("get_url", &["dest", "url"]),
    surface(
        "git",
        &[
            "accept_hostkey",
            "dest",
            "force",
            "repo",
            "update",
            "version",
        ],
    ),
    ModuleSurface {
        literal_enums: &[("state", &["present", "absent"])],
        ..surface("group", &["gid", "name", "state"])
    },
    ModuleSurface {
        literal_enums: &[("jump", &["DROP"])],
        ..surface(
            "iptables",
            &[
                "chain",
                "comment",
                "destination",
                "ip_version",
                "jump",
                "out_interface",
                "policy",
                "protocol",
            ],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface("lineinfile", &["line", "path", "regexp", "state"])
    },
    surface("pause", &["prompt"]),
    surface("replace", &["path", "regexp", "replace"]),
    ModuleSurface {
        literal_enums: &[("state", &["started", "restarted"])],
        ..surface("service", &["enabled", "name", "state"])
    },
    ModuleSurface {
        any_params: true,
        ..surface("set_fact", &[])
    },
    ModuleSurface {
        free_form: true,
        args_params: &["chdir", "creates", "executable"],
        ..surface("shell", &[])
    },
    surface("slurp", &["src"]),
    surface("stat", &["follow", "path"]),
    ModuleSurface {
        literal_enums: &[("state", &["present"])],
        ..surface(
            "sysctl",
            &[
                "name",
                "reload",
                "state",
                "sysctl_file",
                "sysctl_set",
                "value",
            ],
        )
    },
    ModuleSurface {
        literal_enums: &[("state", &["started", "stopped", "restarted"])],
        ..surface("systemd", &["daemon_reload", "enabled", "name", "state"])
    },
    surface("template", &["dest", "group", "mode", "owner", "src"]),
    ModuleSurface {
        literal_enums: &[("state", &["absent"])],
        ..surface(
            "user",
            &[
                "append",
                "comment",
                "create_home",
                "group",
                "groups",
                "home",
                "name",
                "remove",
                "shell",
                "state",
                "system",
                "uid",
            ],
        )
    },
];

pub fn lookup(name: &str) -> Option<&'static ModuleSurface> {
    MODULES.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_sorted_and_unique() {
        let names: Vec<_> = MODULES.iter().map(|m| m.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted, "MODULES must stay sorted and unique");
    }

    #[test]
    fn closed_surface_has_exactly_the_extracted_modules() {
        assert_eq!(MODULES.len(), 36);
        assert!(lookup("apt").is_some());
        assert!(lookup("community.postgresql.postgresql_privs").is_some());
        assert!(
            lookup("ansible.builtin.apt").is_none(),
            "FQCN builtin spelling is not in the workload"
        );
        assert!(lookup("dnf").is_none());
    }
}
