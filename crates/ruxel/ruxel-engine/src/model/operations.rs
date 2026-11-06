use super::{Field, FieldType, Operation};

pub(super) const OPERATIONS: &[Operation] = &[
    Operation {
        kind: "apt",
        fields: &[
            Field {
                name: "autoremove",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "force",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "name",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present", "latest"]),
            },
            Field {
                name: "update_cache",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "upgrade",
                required: false,
                kind: FieldType::Choice(&["dist"]),
            },
        ],
    },
    Operation {
        kind: "apt_repository",
        fields: &[
            Field {
                name: "filename",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "repo",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
            Field {
                name: "update_cache",
                required: false,
                kind: FieldType::Flag,
            },
        ],
    },
    Operation {
        kind: "authorized_key",
        fields: &[
            Field {
                name: "key",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
            Field {
                name: "user",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "blockinfile",
        fields: &[
            Field {
                name: "block",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "create",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "group",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "mode",
                required: false,
                kind: FieldType::Mode,
            },
            Field {
                name: "owner",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "path",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "command",
        fields: &[
            Field {
                name: "argv",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "chdir",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "cmd",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "creates",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "removes",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "copy",
        fields: &[
            Field {
                name: "content",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "dest",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "force",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "group",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "mode",
                required: false,
                kind: FieldType::Mode,
            },
            Field {
                name: "owner",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "src",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "file",
        fields: &[
            Field {
                name: "dest",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "group",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "mode",
                required: false,
                kind: FieldType::Mode,
            },
            Field {
                name: "owner",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "path",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "recurse",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "src",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["file", "directory", "absent", "link"]),
            },
        ],
    },
    Operation {
        kind: "filesystem",
        fields: &[
            Field {
                name: "dev",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "fstype",
                required: true,
                kind: FieldType::Choice(&["ext4", "xfs"]),
            },
            Field {
                name: "resizefs",
                required: false,
                kind: FieldType::Flag,
            },
        ],
    },
    Operation {
        kind: "get_url",
        fields: &[
            Field {
                name: "dest",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "url",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "git",
        fields: &[
            Field {
                name: "accept_hostkey",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "dest",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "force",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "repo",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "update",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "version",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "group",
        fields: &[
            Field {
                name: "gid",
                required: false,
                kind: FieldType::Number,
            },
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present", "absent"]),
            },
        ],
    },
    Operation {
        kind: "iptables",
        fields: &[
            Field {
                name: "chain",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "comment",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "destination",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "ip_version",
                required: false,
                kind: FieldType::Choice(&["ipv4", "ipv6"]),
            },
            Field {
                name: "jump",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "out_interface",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "policy",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "protocol",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "lineinfile",
        fields: &[
            Field {
                name: "line",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "path",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "regexp",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present", "absent"]),
            },
        ],
    },
    Operation {
        kind: "replace",
        fields: &[
            Field {
                name: "path",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "regexp",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "replace",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "shell",
        fields: &[
            Field {
                name: "argv",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "chdir",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "cmd",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "creates",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "executable",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "removes",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "slurp",
        fields: &[Field {
            name: "src",
            required: true,
            kind: FieldType::Text,
        }],
    },
    Operation {
        kind: "stat",
        fields: &[
            Field {
                name: "follow",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "path",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "sysctl",
        fields: &[
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "reload",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
            Field {
                name: "sysctl_file",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "sysctl_set",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "value",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "systemd",
        fields: &[
            Field {
                name: "daemon_reload",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "enabled",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "name",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["started", "stopped", "restarted"]),
            },
        ],
    },
    Operation {
        kind: "user",
        fields: &[
            Field {
                name: "append",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "comment",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "create_home",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "group",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "groups",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "home",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "remove",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "shell",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present", "absent"]),
            },
            Field {
                name: "system",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "uid",
                required: false,
                kind: FieldType::Number,
            },
        ],
    },
    Operation {
        kind: "community.general.lvg",
        fields: &[
            Field {
                name: "pvs",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "vg",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "community.general.lvol",
        fields: &[
            Field {
                name: "lv",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "resizefs",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "size",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "vg",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "community.general.timezone",
        fields: &[
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "ansible.posix.mount",
        fields: &[
            Field {
                name: "fstype",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "opts",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "path",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "src",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["mounted"]),
            },
        ],
    },
    Operation {
        kind: "community.postgresql.postgresql_db",
        fields: &[
            Field {
                name: "login_db",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "login_port",
                required: false,
                kind: FieldType::Port,
            },
            Field {
                name: "login_user",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "owner",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
        ],
    },
    Operation {
        kind: "community.postgresql.postgresql_user",
        fields: &[
            Field {
                name: "login_db",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "login_port",
                required: false,
                kind: FieldType::Port,
            },
            Field {
                name: "login_user",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "password",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "role_attr_flags",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
        ],
    },
    Operation {
        kind: "community.postgresql.postgresql_schema",
        fields: &[
            Field {
                name: "login_db",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "login_port",
                required: false,
                kind: FieldType::Port,
            },
            Field {
                name: "login_user",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "name",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "owner",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
        ],
    },
    Operation {
        kind: "community.postgresql.postgresql_privs",
        fields: &[
            Field {
                name: "login_db",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "login_port",
                required: false,
                kind: FieldType::Port,
            },
            Field {
                name: "login_user",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "objs",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "privs",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "role",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "schema",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "state",
                required: false,
                kind: FieldType::Choice(&["present"]),
            },
            Field {
                name: "target_roles",
                required: false,
                kind: FieldType::TextList,
            },
            Field {
                name: "type",
                required: true,
                kind: FieldType::Choice(&["database", "schema", "table", "default_privs"]),
            },
        ],
    },
    Operation {
        kind: "template",
        fields: &[
            Field {
                name: "dest",
                required: true,
                kind: FieldType::Text,
            },
            Field {
                name: "force",
                required: false,
                kind: FieldType::Flag,
            },
            Field {
                name: "group",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "mode",
                required: false,
                kind: FieldType::Mode,
            },
            Field {
                name: "owner",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "src",
                required: true,
                kind: FieldType::Text,
            },
        ],
    },
    Operation {
        kind: "assert",
        fields: &[
            Field {
                name: "fail_msg",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "that",
                required: true,
                kind: FieldType::TextList,
            },
        ],
    },
    Operation {
        kind: "debug",
        fields: &[Field {
            name: "msg",
            required: false,
            kind: FieldType::Text,
        }],
    },
    Operation {
        kind: "fail",
        fields: &[Field {
            name: "msg",
            required: false,
            kind: FieldType::Text,
        }],
    },
    Operation {
        kind: "pause",
        fields: &[
            Field {
                name: "prompt",
                required: false,
                kind: FieldType::Text,
            },
            Field {
                name: "seconds",
                required: false,
                kind: FieldType::Number,
            },
            Field {
                name: "minutes",
                required: false,
                kind: FieldType::Number,
            },
        ],
    },
    Operation {
        kind: "set_fact",
        fields: &[Field {
            name: "values",
            required: false,
            kind: FieldType::ExprMap,
        }],
    },
];
