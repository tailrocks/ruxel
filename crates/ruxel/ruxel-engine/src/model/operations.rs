use super::{Field, FieldType, Operation};

macro_rules! operation {
    ($kind:literal; $($name:literal => $required:literal, $field:expr;)*) => {
        Operation { kind: $kind, fields: &[$(Field {name:$name, required:$required, kind:$field}),*] }
    };
}

pub(super) const OPERATIONS: &[Operation] = &[
    operation!("apt";
        "autoremove" => false, FieldType::Flag;
        "force" => false, FieldType::Flag;
        "name" => false, FieldType::TextList;
        "state" => false, FieldType::Choice(&["present","latest"]);
        "update_cache" => false, FieldType::Flag;
        "upgrade" => false, FieldType::Choice(&["dist"]);
    ),
    operation!("apt_repository";
        "filename" => true, FieldType::Text;
        "repo" => true, FieldType::Text;
        "state" => false, FieldType::Choice(&["present"]);
        "update_cache" => false, FieldType::Flag;
    ),
    operation!("authorized_key";
        "key" => true, FieldType::Text;
        "state" => false, FieldType::Choice(&["present"]);
        "user" => true, FieldType::Text;
    ),
    operation!("blockinfile";
        "block" => true, FieldType::Text;
        "create" => false, FieldType::Flag;
        "group" => false, FieldType::Text;
        "mode" => false, FieldType::Mode;
        "owner" => false, FieldType::Text;
        "path" => true, FieldType::Text;
    ),
    operation!("command";
        "argv" => false, FieldType::TextList;
        "chdir" => false, FieldType::Text;
        "cmd" => false, FieldType::Text;
        "creates" => false, FieldType::Text;
    ),
    operation!("copy";
        "content" => false, FieldType::Text;
        "dest" => true, FieldType::Text;
        "force" => false, FieldType::Flag;
        "group" => false, FieldType::Text;
        "mode" => false, FieldType::Mode;
        "owner" => false, FieldType::Text;
        "src" => false, FieldType::Text;
    ),
    operation!("file";
        "dest" => false, FieldType::Text;
        "group" => false, FieldType::Text;
        "mode" => false, FieldType::Mode;
        "owner" => false, FieldType::Text;
        "path" => false, FieldType::Text;
        "recurse" => false, FieldType::Flag;
        "src" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["file","directory","absent","link"]);
    ),
    operation!("filesystem";
        "dev" => true, FieldType::Text;
        "fstype" => true, FieldType::Choice(&["ext4","xfs"]);
        "resizefs" => false, FieldType::Flag;
    ),
    operation!("get_url";
        "dest" => true, FieldType::Text;
        "url" => true, FieldType::Text;
    ),
    operation!("git";
        "accept_hostkey" => false, FieldType::Flag;
        "dest" => true, FieldType::Text;
        "force" => false, FieldType::Flag;
        "repo" => true, FieldType::Text;
        "update" => false, FieldType::Flag;
        "version" => false, FieldType::Text;
    ),
    operation!("group";
        "gid" => false, FieldType::Number;
        "name" => true, FieldType::Text;
        "state" => false, FieldType::Choice(&["present","absent"]);
    ),
    operation!("iptables";
        "chain" => true, FieldType::Text;
        "comment" => false, FieldType::Text;
        "destination" => false, FieldType::Text;
        "ip_version" => false, FieldType::Choice(&["ipv4","ipv6"]);
        "jump" => false, FieldType::Text;
        "out_interface" => false, FieldType::Text;
        "policy" => false, FieldType::Text;
        "protocol" => false, FieldType::Text;
    ),
    operation!("lineinfile";
        "line" => false, FieldType::Text;
        "path" => true, FieldType::Text;
        "regexp" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["present","absent"]);
    ),
    operation!("replace";
        "path" => true, FieldType::Text;
        "regexp" => true, FieldType::Text;
        "replace" => false, FieldType::Text;
    ),
    operation!("shell";
        "chdir" => false, FieldType::Text;
        "cmd" => true, FieldType::Text;
        "creates" => false, FieldType::Text;
        "executable" => false, FieldType::Text;
    ),
    operation!("slurp";
        "src" => true, FieldType::Text;
    ),
    operation!("stat";
        "follow" => false, FieldType::Flag;
        "path" => true, FieldType::Text;
    ),
    operation!("sysctl";
        "name" => true, FieldType::Text;
        "reload" => false, FieldType::Flag;
        "state" => false, FieldType::Choice(&["present"]);
        "sysctl_file" => false, FieldType::Text;
        "sysctl_set" => false, FieldType::Flag;
        "value" => true, FieldType::Scalar;
    ),
    operation!("systemd";
        "daemon_reload" => false, FieldType::Flag;
        "enabled" => false, FieldType::Flag;
        "name" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["started","stopped","restarted"]);
    ),
    operation!("user";
        "append" => false, FieldType::Flag;
        "comment" => false, FieldType::Text;
        "create_home" => false, FieldType::Flag;
        "group" => false, FieldType::Text;
        "groups" => false, FieldType::TextList;
        "home" => false, FieldType::Text;
        "name" => true, FieldType::Text;
        "remove" => false, FieldType::Flag;
        "shell" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["present","absent"]);
        "system" => false, FieldType::Flag;
        "uid" => false, FieldType::Number;
    ),
    operation!("community.general.lvg";
        "pvs" => true, FieldType::TextList;
        "vg" => true, FieldType::Text;
    ),
    operation!("community.general.lvol";
        "lv" => true, FieldType::Text;
        "resizefs" => false, FieldType::Flag;
        "size" => true, FieldType::Text;
        "vg" => true, FieldType::Text;
    ),
    operation!("community.general.timezone";
        "name" => true, FieldType::Text;
    ),
    operation!("ansible.posix.mount";
        "fstype" => true, FieldType::Text;
        "opts" => false, FieldType::Text;
        "path" => true, FieldType::Text;
        "src" => true, FieldType::Text;
        "state" => false, FieldType::Choice(&["mounted"]);
    ),
    operation!("community.postgresql.postgresql_db";
        "login_port" => false, FieldType::Port;
        "login_user" => false, FieldType::Text;
        "name" => true, FieldType::Text;
        "owner" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["present"]);
    ),
    operation!("community.postgresql.postgresql_user";
        "login_port" => false, FieldType::Port;
        "login_user" => false, FieldType::Text;
        "name" => true, FieldType::Text;
        "password" => false, FieldType::Text;
        "role_attr_flags" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["present"]);
    ),
    operation!("community.postgresql.postgresql_schema";
        "login_db" => true, FieldType::Text;
        "login_port" => false, FieldType::Port;
        "login_user" => false, FieldType::Text;
        "name" => true, FieldType::Text;
        "owner" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["present"]);
    ),
    operation!("community.postgresql.postgresql_privs";
        "login_db" => true, FieldType::Text;
        "login_port" => false, FieldType::Port;
        "login_user" => false, FieldType::Text;
        "objs" => false, FieldType::TextList;
        "privs" => false, FieldType::TextList;
        "role" => true, FieldType::Text;
        "schema" => false, FieldType::Text;
        "state" => false, FieldType::Choice(&["present"]);
        "target_roles" => false, FieldType::TextList;
        "type" => true, FieldType::Choice(&["database","schema","table","default_privs"]);
    ),
    operation!("template";
        "content" => false, FieldType::Text;
        "dest" => true, FieldType::Text;
        "force" => false, FieldType::Flag;
        "group" => false, FieldType::Text;
        "mode" => false, FieldType::Mode;
        "owner" => false, FieldType::Text;
        "src" => false, FieldType::Text;
    ),
    operation!("assert";
        "fail_msg" => false, FieldType::Text;
        "that" => true, FieldType::Flags;
    ),
    operation!("debug";
        "msg" => false, FieldType::Text;
    ),
    operation!("fail";
        "msg" => false, FieldType::Text;
    ),
    operation!("pause";
        "prompt" => false, FieldType::Text;
        "seconds" => false, FieldType::Number;
        "minutes" => false, FieldType::Number;
    ),
    operation!("set_fact";
        "values" => false, FieldType::ExprMap;
    ),
];
