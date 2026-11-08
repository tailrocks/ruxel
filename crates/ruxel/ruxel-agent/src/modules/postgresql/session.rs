use super::*;

pub(super) fn login_port(obj: &serde_json::Map<String, Value>) -> String {
    obj.get("login_port").map_or_else(
        || "5432".into(),
        |v| match v {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => "5432".into(),
        },
    )
}

pub(super) struct PgContext<'a> {
    pub(super) exec: &'a ExecContext,
    pub(super) login_user: Option<&'a str>,
    pub(super) login_db: Option<&'a str>,
}

impl std::ops::Deref for PgContext<'_> {
    type Target = ExecContext;

    fn deref(&self) -> &Self::Target {
        self.exec
    }
}

pub(super) fn psql(
    ctx: &PgContext<'_>,
    port: &str,
    db: Option<&str>,
    sql: &str,
) -> Result<String, String> {
    let key = SessionKey {
        become_user: ctx.become_user.clone(),
        port: port.to_string(),
        database: db.or(ctx.login_db).map(str::to_string),
        login_user: ctx.login_user.map(str::to_owned),
        environment: ctx.environment.iter().cloned().collect(),
    };
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        if !sessions.contains_key(&key) {
            sessions.insert(key.clone(), PsqlSession::spawn(ctx.exec, &key)?);
        }
        let result = sessions.get_mut(&key).expect("session inserted").query(sql);
        if result.is_err() {
            sessions.remove(&key);
        }
        result
    })
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct SessionKey {
    become_user: Option<String>,
    port: String,
    database: Option<String>,
    login_user: Option<String>,
    environment: std::collections::BTreeMap<String, String>,
}

struct PsqlSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
    stderr_thread: Option<std::thread::JoinHandle<()>>,
    sequence: u64,
}

thread_local! {
    static SESSIONS: RefCell<HashMap<SessionKey, PsqlSession>> = RefCell::new(HashMap::new());
}

pub(crate) fn shutdown() {
    SESSIONS.with(|sessions| sessions.borrow_mut().clear());
}

impl PsqlSession {
    fn spawn(ctx: &ExecContext, key: &SessionKey) -> Result<Self, String> {
        let mut args = vec!["-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-p", &key.port];
        if let Some(user) = &key.login_user {
            args.push("--username");
            args.push(user);
        }
        let database_arg = key.database.as_ref().map(|database| {
            if database.is_empty() {
                String::new()
            } else {
                format!(
                    "dbname='{}'",
                    database.replace('\\', "\\\\").replace('\'', "\\'")
                )
            }
        });
        if let Some(database) = &database_arg {
            args.push("-d");
            args.push(database);
        }
        let mut child = super::super::subprocess::psql_command(ctx, &args)?
            .spawn()
            .map_err(|error| format!("exec psql: {error}"))?;
        let stdin = child.stdin.take().ok_or("psql stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("psql stdout")?);
        let mut child_stderr = child.stderr.take().ok_or("psql stderr")?;
        let stderr = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&stderr);
        let stderr_thread = std::thread::spawn(move || {
            use std::io::Read as _;
            let mut text = String::new();
            let _ = child_stderr.read_to_string(&mut text);
            *captured.lock().unwrap() = text;
        });
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
            stderr_thread: Some(stderr_thread),
            sequence: 0,
        })
    }

    fn query(&mut self, sql: &str) -> Result<String, String> {
        self.sequence += 1;
        let marker = format!(
            "__RUXEL_QUERY_END_{}_{}__",
            std::process::id(),
            self.sequence
        );
        self.stdin
            .write_all(sql.as_bytes())
            .and_then(|()| {
                if sql.trim_end().ends_with(';') {
                    self.stdin.write_all(b"\n")
                } else {
                    self.stdin.write_all(b";\n")
                }
            })
            .and_then(|()| {
                self.stdin
                    .write_all(format!("\\echo {marker}\n").as_bytes())
            })
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("psql stdin: {error}"))?;

        let mut output = String::new();
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("psql stdout: {error}"))?;
            if read == 0 {
                if let Some(thread) = self.stderr_thread.take() {
                    let _ = thread.join();
                }
                let error = self.stderr.lock().unwrap().trim().to_string();
                return Err(if error.is_empty() {
                    "psql exited before query completed".into()
                } else {
                    format!("psql failed: {error}")
                });
            }
            if line.trim_end() == marker {
                return Ok(output.trim().to_string());
            }
            output.push_str(&line);
        }
    }
}

impl Drop for PsqlSession {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"\\q\n");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}
