#![expect(
    clippy::cast_possible_truncation,
    clippy::expect_used,
    clippy::format_push_string,
    clippy::let_underscore_must_use,
    clippy::unwrap_used,
    reason = "replaced by the Ruxel rewrite"
)]

use super::{ExecContext, become_command, params_object, str_param};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Arc, Mutex};

mod encoding;
mod helpers;
mod objects;
mod privileges;
mod session;

use encoding::{b64_decode, b64_encode};
use helpers::{
    acl_privileges, ident, lit, role_flag_tokens, validate_privs, validate_role_attr_flags,
};
pub(super) use objects::{db, schema, user};
#[cfg(test)]
use objects::{scram_stored_key, wanted_flag};
pub(super) use privileges::privs;
#[cfg(test)]
use privileges::{default_priv_query, grant_sql};
pub(super) use session::shutdown;
use session::{PgContext, login_port, psql};
#[cfg(test)]
mod tests;
