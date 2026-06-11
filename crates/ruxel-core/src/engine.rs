//! The templating engine: MiniJinja configured to reproduce ansible-core
//! 2.21's observable rendering semantics for the closed surface in
//! docs/SEMANTICS.md §2 — native types, the workload's filter set, lazy
//! variable resolution, and controller-side lookups with a dry-secrets mode.
//!
//! Every semantic choice here is provisional until the render-parity harness
//! (tools/oracle/render_parity.py) confirms it byte-for-byte against the
//! pinned oracle; the goldens, not this file's comments, are the contract.

use minijinja::value::{Kwargs, Object, Value};
use minijinja::{Environment, ErrorKind as MjErrorKind, UndefinedBehavior};
use serde_norway::Value as Yaml;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("template error: {0}")]
    Template(#[from] minijinja::Error),
    #[error("variable cycle while rendering {0:?}")]
    VarCycle(String),
    #[error("undefined variable in {0:?}")]
    Undefined(String),
    #[error("lookup {0:?} failed: {1}")]
    Lookup(String, String),
    #[error("unsupported YAML value (tagged) in template scope")]
    TaggedValue,
}

// -- Lookups ----------------------------------------------------------------

/// Controller-side lookup resolution (SEMANTICS §2). Implementations:
/// `DrySecrets` (deterministic fakes, used by tests and `--dry-secrets`),
/// and later a real `op`-backed resolver (M3).
pub trait LookupResolver: Send + Sync {
    fn onepassword(
        &self,
        item: &str,
        field: Option<&str>,
        vault: Option<&str>,
        section: Option<&str>,
    ) -> Result<String, String>;
    fn pipe(&self, cmd: &str) -> Result<String, String>;
}

/// Canonical identity of one lookup invocation; memoization and the
/// dry-secrets fake derive from this exact string. The Python fake lookup
/// plugins in tools/oracle/ must build the identical string.
fn lookup_key(kind: &str, parts: &[&str]) -> String {
    let mut key = String::from(kind);
    for p in parts {
        key.push('\u{1f}');
        key.push_str(p);
    }
    key
}

fn dry_value(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("dry-secret-{hex}")
}

/// Deterministic fake secrets: stable across runs and languages so parity
/// goldens are reproducible without any real secret store.
pub struct DrySecrets;

impl LookupResolver for DrySecrets {
    fn onepassword(
        &self,
        item: &str,
        field: Option<&str>,
        vault: Option<&str>,
        section: Option<&str>,
    ) -> Result<String, String> {
        let key = lookup_key(
            "onepassword",
            &[
                item,
                field.unwrap_or(""),
                vault.unwrap_or(""),
                section.unwrap_or(""),
            ],
        );
        Ok(dry_value(&key))
    }

    fn pipe(&self, cmd: &str) -> Result<String, String> {
        Ok(dry_value(&lookup_key("pipe", &[cmd])))
    }
}

/// Memoizes each distinct lookup invocation once per run (the specified
/// deviation in SEMANTICS §2: one consistent secret snapshot per run).
pub struct MemoizedResolver<R> {
    inner: R,
    cache: Mutex<HashMap<String, String>>,
}

impl<R: LookupResolver> MemoizedResolver<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn memo(
        &self,
        key: String,
        fetch: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            return Ok(hit.clone());
        }
        let value = fetch()?;
        self.cache.lock().unwrap().insert(key, value.clone());
        Ok(value)
    }
}

impl<R: LookupResolver> LookupResolver for MemoizedResolver<R> {
    fn onepassword(
        &self,
        item: &str,
        field: Option<&str>,
        vault: Option<&str>,
        section: Option<&str>,
    ) -> Result<String, String> {
        let key = lookup_key(
            "onepassword",
            &[
                item,
                field.unwrap_or(""),
                vault.unwrap_or(""),
                section.unwrap_or(""),
            ],
        );
        self.memo(key, || self.inner.onepassword(item, field, vault, section))
    }

    fn pipe(&self, cmd: &str) -> Result<String, String> {
        let key = lookup_key("pipe", &[cmd]);
        self.memo(key, || self.inner.pipe(cmd))
    }
}

// -- Scope ------------------------------------------------------------------

/// One variable binding: either a raw (possibly template-bearing) YAML value
/// rendered lazily on first reference (play vars), or an already-final value
/// (facts, register results, loop `item`).
#[derive(Clone, Debug)]
pub enum VarValue {
    Raw(Yaml),
    Final(Value),
}

/// Layered variable scope, lowest→highest precedence (SEMANTICS §2: play
/// vars → set_fact → register; loop `item` and task vars on top).
#[derive(Clone, Default, Debug)]
pub struct Scope {
    layers: Vec<Arc<Vec<(String, VarValue)>>>,
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a higher-precedence layer; later layers shadow earlier ones.
    pub fn with_layer(&self, vars: Vec<(String, VarValue)>) -> Self {
        let mut layers = self.layers.clone();
        layers.push(Arc::new(vars));
        Self { layers }
    }

    fn get_raw(&self, name: &str) -> Option<&VarValue> {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| layer.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v))
    }

    fn names(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for layer in &self.layers {
            for (k, _) in layer.iter() {
                if seen.insert(k.clone()) {
                    names.push(k.clone());
                }
            }
        }
        names
    }
}

/// The minijinja context object: resolves scope names on demand, rendering
/// raw values through the engine at first access (Ansible's lazy semantics —
/// an unused broken var is never an error) and memoizing per evaluation
/// context.
#[derive(Debug)]
struct ScopeObject {
    engine: Arc<EngineInner>,
    scope: Scope,
    memo: Mutex<HashMap<String, Value>>,
    in_flight: Mutex<HashSet<String>>,
}

impl Object for ScopeObject {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        let name = key.as_str()?;
        if let Some(hit) = self.memo.lock().unwrap().get(name) {
            return Some(hit.clone());
        }
        let raw = self.scope.get_raw(name)?.clone();
        let rendered = match raw {
            VarValue::Final(v) => v,
            VarValue::Raw(yaml) => {
                if !self.in_flight.lock().unwrap().insert(name.to_string()) {
                    return Some(Value::from(format!("[ruxel: variable cycle at {name}]")));
                }
                let result = render_yaml(&self.engine, &yaml, self);
                self.in_flight.lock().unwrap().remove(name);
                match result {
                    Ok(v) => v,
                    // Surface render errors at use-site, like Ansible's
                    // lazy templating does: the error message becomes the
                    // failure when the variable is actually consumed.
                    Err(e) => {
                        return Some(Value::from_object(ScopeRenderError {
                            name: name.to_string(),
                            message: e.to_string(),
                        }));
                    }
                }
            }
        };
        self.memo
            .lock()
            .unwrap()
            .insert(name.to_string(), rendered.clone());
        Some(rendered)
    }

    fn enumerate(self: &Arc<Self>) -> minijinja::value::Enumerator {
        minijinja::value::Enumerator::Values(
            self.scope.names().into_iter().map(Value::from).collect(),
        )
    }
}

/// A poisoned value: referencing a variable whose lazy render failed turns
/// into a hard template error the moment it is used in any operation.
#[derive(Debug)]
struct ScopeRenderError {
    name: String,
    message: String,
}

impl Object for ScopeRenderError {
    fn get_value(self: &Arc<Self>, _key: &Value) -> Option<Value> {
        None
    }

    fn render(self: &Arc<Self>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    where
        Self: Sized + 'static,
    {
        write!(
            f,
            "[ruxel: error rendering {}: {}]",
            self.name, self.message
        )
    }
}

// -- Engine -----------------------------------------------------------------

#[derive(Debug)]
struct EngineInner {
    env: Environment<'static>,
}

pub struct Engine {
    inner: Arc<EngineInner>,
}

impl Engine {
    pub fn new(resolver: Arc<dyn LookupResolver>) -> Self {
        let mut env = Environment::new();
        // Ansible's undefined chains through attribute access and is caught
        // by `default` (the workload's `item.stat.exists | default(false)`
        // pattern). ⚠ pinned by the parity harness.
        env.set_undefined_behavior(UndefinedBehavior::Chainable);
        // …but *emitting* an undefined into rendered output is
        // AnsibleUndefinedVariable (pinned 2026-06-11: config/sentry/
        // config.yml's slack_* refs error under the oracle).
        env.set_formatter(|out, _state, value| {
            if value.is_undefined() {
                return Err(minijinja::Error::new(
                    MjErrorKind::UndefinedError,
                    "undefined value in template output",
                ));
            }
            write!(out, "{value}").map_err(minijinja::Error::from)
        });
        // The template module's keep_trailing_newline=True default
        // (SEMANTICS §6 template). ⚠ pinned by the 22-template parity gate.
        env.set_keep_trailing_newline(true);

        env.add_filter("bool", filter_bool);
        env.add_filter("hash", filter_hash);
        env.add_filter("subelements", filter_subelements);
        env.add_filter("b64decode", filter_b64decode);

        let lookup_resolver = resolver.clone();
        env.add_function(
            "lookup",
            move |name: String, term: String, kwargs: Kwargs| -> Result<Value, minijinja::Error> {
                lookup_fn(&*lookup_resolver, &name, &term, &kwargs).map(Value::from)
            },
        );

        Self {
            inner: Arc::new(EngineInner { env }),
        }
    }

    fn scope_object(&self, scope: &Scope) -> Arc<ScopeObject> {
        Arc::new(ScopeObject {
            engine: self.inner.clone(),
            scope: scope.clone(),
            memo: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
        })
    }

    fn context(&self, scope: &Scope) -> Value {
        Value::from_dyn_object(self.scope_object(scope))
    }

    /// Render a string with Ansible native-types semantics: a string that is
    /// exactly one `{{ expression }}` evaluates to the native value; anything
    /// else containing template syntax renders to a string; a plain string
    /// passes through untouched.
    pub fn render_str(&self, template: &str, scope: &Scope) -> Result<Value, EngineError> {
        render_str_inner(&self.inner, template, &self.context(scope))
    }

    /// Render a template file's content the way the `template` module does
    /// (trailing newline preserved). Always a string.
    pub fn render_template_file(
        &self,
        content: &str,
        scope: &Scope,
    ) -> Result<String, EngineError> {
        Ok(self.inner.env.render_str(content, self.context(scope))?)
    }

    /// Render an arbitrary parsed-YAML value: strings render with native
    /// semantics, containers recurse, scalars pass through.
    pub fn render_value(&self, value: &Yaml, scope: &Scope) -> Result<Value, EngineError> {
        let obj = self.scope_object(scope);
        render_yaml(&self.inner, value, &obj)
    }

    /// Evaluate a task condition (`when`/`until`/`changed_when`/
    /// `failed_when`): bare expression(s), Ansible truthiness; a list is the
    /// AND of all entries (SEMANTICS §3.2).
    pub fn eval_condition(
        &self,
        cond: &crate::playbook::Condition,
        scope: &Scope,
    ) -> Result<bool, EngineError> {
        use crate::playbook::Condition;
        let ctx = self.context(scope);
        match cond {
            Condition::Literal(b) => Ok(*b),
            Condition::Expr(e) => eval_expr_bool(&self.inner, e, &ctx),
            Condition::All(exprs) => {
                for e in exprs {
                    if !eval_expr_bool(&self.inner, e, &ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

fn eval_expr_bool(inner: &EngineInner, expr: &str, ctx: &Value) -> Result<bool, EngineError> {
    let compiled = inner.env.compile_expression(expr)?;
    let value = compiled.eval(ctx.clone())?;
    // Ansible raises AnsibleUndefinedVariable when a condition's result is
    // undefined (pinned 2026-06-11 against the 2.21 oracle) — chainable
    // undefined is only allowed to survive *inside* an expression.
    if value.is_undefined() {
        return Err(EngineError::Undefined(expr.to_string()));
    }
    Ok(value.is_true())
}

/// `"{{ expr }}"` (exactly one expression, nothing else) → Some(expr).
fn single_expression(template: &str) -> Option<&str> {
    let inner = template.strip_prefix("{{")?.strip_suffix("}}")?;
    if inner.contains("{{") || inner.contains("}}") || inner.contains("{%") {
        return None;
    }
    Some(inner)
}

fn render_str_inner(
    inner: &EngineInner,
    template: &str,
    ctx: &Value,
) -> Result<Value, EngineError> {
    if !template.contains("{{") && !template.contains("{%") {
        return Ok(Value::from(template));
    }
    if let Some(expr) = single_expression(template) {
        let compiled = inner.env.compile_expression(expr)?;
        let value = compiled.eval(ctx.clone())?;
        // Match AnsibleUndefinedVariable on an undefined final result
        // (pinned 2026-06-11): chaining may pass through undefined, but a
        // template must not silently *produce* it.
        if value.is_undefined() {
            return Err(EngineError::Undefined(template.to_string()));
        }
        return Ok(value);
    }
    Ok(Value::from(inner.env.render_str(template, ctx.clone())?))
}

fn render_yaml(
    inner: &Arc<EngineInner>,
    value: &Yaml,
    scope_obj: &Arc<ScopeObject>,
) -> Result<Value, EngineError> {
    Ok(match value {
        Yaml::Null => Value::from(()),
        Yaml::Bool(b) => Value::from(*b),
        Yaml::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                Value::from(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Yaml::String(s) => {
            let ctx = Value::from_dyn_object(scope_obj.clone());
            render_str_inner(inner, s, &ctx)?
        }
        Yaml::Sequence(items) => Value::from(
            items
                .iter()
                .map(|i| render_yaml(inner, i, scope_obj))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Yaml::Mapping(map) => {
            let mut out: Vec<(Value, Value)> = Vec::with_capacity(map.len());
            for (k, v) in map {
                let key = match k {
                    Yaml::String(s) => Value::from(s.as_str()),
                    other => render_yaml(inner, other, scope_obj)?,
                };
                out.push((key, render_yaml(inner, v, scope_obj)?));
            }
            Value::from_iter(out)
        }
        Yaml::Tagged(_) => return Err(EngineError::TaggedValue),
    })
}

// -- Filters ----------------------------------------------------------------

/// Ansible's `bool` filter (core 2.21 strict semantics): real bools pass,
/// the classic string/int spellings convert, anything else is an error.
/// ⚠ exact accepted set pinned by harness experiments.
fn filter_bool(value: Value) -> Result<bool, minijinja::Error> {
    if value.is_undefined() || value.is_none() {
        return Err(minijinja::Error::new(
            MjErrorKind::InvalidOperation,
            "bool filter: cannot convert None/undefined to bool",
        ));
    }
    if let Some(b) = bool_of(&value) {
        return Ok(b);
    }
    Err(minijinja::Error::new(
        MjErrorKind::InvalidOperation,
        format!("bool filter: {value:?} is not a valid boolean"),
    ))
}

fn bool_of(value: &Value) -> Option<bool> {
    if let Ok(b) = bool::try_from(value.clone()) {
        return Some(b);
    }
    if let Some(s) = value.as_str() {
        return match s.to_ascii_lowercase().as_str() {
            "yes" | "on" | "1" | "true" => Some(true),
            "no" | "off" | "0" | "false" => Some(false),
            _ => None,
        };
    }
    if let Ok(i) = i64::try_from(value.clone()) {
        return match i {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        };
    }
    None
}

/// Ansible's `hash` filter; the workload uses only `hash('sha256')`
/// (SEMANTICS §2), so anything else is outside the closed surface.
fn filter_hash(value: Value, algorithm: String) -> Result<String, minijinja::Error> {
    if algorithm != "sha256" {
        return Err(minijinja::Error::new(
            MjErrorKind::InvalidOperation,
            format!(
                "hash filter: algorithm {algorithm:?} is outside the closed surface (only sha256)"
            ),
        ));
    }
    let Some(s) = value.as_str() else {
        return Err(minijinja::Error::new(
            MjErrorKind::InvalidOperation,
            "hash filter: only string input is in the closed surface",
        ));
    };
    let digest = Sha256::digest(s.as_bytes());
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Ansible's `b64decode` filter (setup-sentry.yml's bootstrap-marker
/// compare). UTF-8 output only — the workload decodes a slurped text file.
fn filter_b64decode(value: Value) -> Result<String, minijinja::Error> {
    use base64::Engine as _;
    let Some(s) = value.as_str() else {
        return Err(minijinja::Error::new(
            MjErrorKind::InvalidOperation,
            "b64decode: input must be a string",
        ));
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| {
            minijinja::Error::new(MjErrorKind::InvalidOperation, format!("b64decode: {e}"))
        })?;
    String::from_utf8(bytes).map_err(|e| {
        minijinja::Error::new(MjErrorKind::InvalidOperation, format!("b64decode: {e}"))
    })
}

/// Ansible's `subelements` filter: list of dicts × subelement list key →
/// list of `[parent, subelement]` pairs. `skip_missing` defaults to false:
/// a parent without the key (or with a non-list value) is an error.
fn filter_subelements(value: Value, key: String) -> Result<Value, minijinja::Error> {
    let mut out: Vec<Value> = Vec::new();
    let iter = value.try_iter().map_err(|_| {
        minijinja::Error::new(
            MjErrorKind::InvalidOperation,
            "subelements: input must be a list of mappings",
        )
    })?;
    for parent in iter {
        let sub = parent.get_attr(&key)?;
        if sub.is_undefined() {
            return Err(minijinja::Error::new(
                MjErrorKind::InvalidOperation,
                format!("subelements: key {key:?} missing from {parent:?}"),
            ));
        }
        let sub_iter = sub.try_iter().map_err(|_| {
            minijinja::Error::new(
                MjErrorKind::InvalidOperation,
                format!("subelements: {key:?} is not a list in {parent:?}"),
            )
        })?;
        for s in sub_iter {
            out.push(Value::from(vec![parent.clone(), s]));
        }
    }
    Ok(Value::from(out))
}

// -- Lookup function ----------------------------------------------------------

fn lookup_fn(
    resolver: &dyn LookupResolver,
    name: &str,
    term: &str,
    kwargs: &Kwargs,
) -> Result<String, minijinja::Error> {
    let result = match name {
        "community.general.onepassword" => {
            let field: Option<String> = kwargs.get("field")?;
            let vault: Option<String> = kwargs.get("vault")?;
            let section: Option<String> = kwargs.get("section")?;
            kwargs.assert_all_used()?;
            resolver.onepassword(term, field.as_deref(), vault.as_deref(), section.as_deref())
        }
        "pipe" => {
            kwargs.assert_all_used()?;
            resolver.pipe(term)
        }
        other => {
            return Err(minijinja::Error::new(
                MjErrorKind::InvalidOperation,
                format!(
                    "lookup plugin {other:?} is outside the closed surface (docs/SEMANTICS.md §2)"
                ),
            ));
        }
    };
    result.map_err(|e| {
        minijinja::Error::new(
            MjErrorKind::InvalidOperation,
            format!("lookup({name:?}, {term:?}) failed: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playbook::Condition;

    fn engine() -> Engine {
        Engine::new(Arc::new(MemoizedResolver::new(DrySecrets)))
    }

    fn scope(pairs: &[(&str, serde_json::Value)]) -> Scope {
        Scope::new().with_layer(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), VarValue::Final(Value::from_serialize(v))))
                .collect(),
        )
    }

    fn raw_scope(yaml: &str) -> Scope {
        let map: Yaml = serde_norway::from_str(yaml).unwrap();
        let Yaml::Mapping(m) = map else {
            panic!("vars yaml must be a mapping")
        };
        Scope::new().with_layer(
            m.into_iter()
                .map(|(k, v)| {
                    let Yaml::String(name) = k else {
                        panic!("string keys")
                    };
                    (name, VarValue::Raw(v))
                })
                .collect(),
        )
    }

    #[test]
    fn plain_string_passes_through() {
        let v = engine()
            .render_str("no templates here", &scope(&[]))
            .unwrap();
        assert_eq!(v.as_str(), Some("no templates here"));
    }

    #[test]
    fn single_expression_yields_native_list() {
        let s = scope(&[("xs", serde_json::json!(["a", "b"]))]);
        let v = engine().render_str("{{ xs }}", &s).unwrap();
        let items: Vec<String> = v
            .try_iter()
            .unwrap()
            .map(|i| i.as_str().unwrap().to_string())
            .collect();
        assert_eq!(items, vec!["a", "b"]);
    }

    #[test]
    fn mixed_template_yields_string() {
        let s = scope(&[("n", serde_json::json!(3))]);
        let v = engine().render_str("count: {{ n }}", &s).unwrap();
        assert_eq!(v.as_str(), Some("count: 3"));
    }

    #[test]
    fn lazy_var_renders_on_reference() {
        let s = raw_scope(
            r#"
a: "{{ b }}-suffix"
b: "base"
broken: "{{ undefined_thing.attr.deep }}"
"#,
        );
        let v = engine().render_str("{{ a }}", &s).unwrap();
        assert_eq!(v.as_str(), Some("base-suffix"));
        // `broken` is never referenced — lazy evaluation means no error.
    }

    #[test]
    fn registered_attribute_access() {
        let s = scope(&[("docker_list", serde_json::json!({"stat": {"exists": true}}))]);
        assert!(
            engine()
                .eval_condition(&Condition::Expr("docker_list.stat.exists".into()), &s)
                .unwrap()
        );
        assert!(
            !engine()
                .eval_condition(&Condition::Expr("not docker_list.stat.exists".into()), &s)
                .unwrap()
        );
    }

    #[test]
    fn condition_list_is_and() {
        let s = scope(&[("x", serde_json::json!(5))]);
        let cond = Condition::All(vec!["x > 1".into(), "x < 3".into()]);
        assert!(!engine().eval_condition(&cond, &s).unwrap());
    }

    #[test]
    fn default_chain_on_missing_attr() {
        let s = scope(&[("item", serde_json::json!({"item": "/dev/disk/by-id/x"}))]);
        let cond = Condition::Expr("item.stat.exists | default(false) | bool".into());
        assert!(!engine().eval_condition(&cond, &s).unwrap());
    }

    #[test]
    fn in_and_not_in_operators() {
        let s = scope(&[("r", serde_json::json!({"rc": 1, "stdout": "x installed y"}))]);
        assert!(
            engine()
                .eval_condition(&Condition::Expr("'installed' in r.stdout".into()), &s)
                .unwrap()
        );
        assert!(
            !engine()
                .eval_condition(&Condition::Expr("r.rc not in [0, 1]".into()), &s)
                .unwrap()
        );
    }

    #[test]
    fn map_attribute_list_pipeline() {
        let s = scope(&[(
            "res",
            serde_json::json!({"results": [{"stdout": "sda"}, {"stdout": "sdb"}]}),
        )]);
        let v = engine()
            .render_str("{{ res.results | map(attribute='stdout') | list }}", &s)
            .unwrap();
        let items: Vec<String> = v
            .try_iter()
            .unwrap()
            .map(|i| i.as_str().unwrap().to_string())
            .collect();
        assert_eq!(items, vec!["sda", "sdb"]);
    }

    #[test]
    fn hash_sha256_matches_python_hashlib() {
        let s = scope(&[("pw", serde_json::json!("secret"))]);
        let v = engine()
            .render_str("{{ pw | hash('sha256') }}", &s)
            .unwrap();
        // python3: hashlib.sha256(b"secret").hexdigest()
        assert_eq!(
            v.as_str(),
            Some("2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b")
        );
    }

    #[test]
    fn hash_other_algorithm_is_error() {
        let s = scope(&[("pw", serde_json::json!("x"))]);
        assert!(engine().render_str("{{ pw | hash('md5') }}", &s).is_err());
    }

    #[test]
    fn subelements_pairs() {
        let s = scope(&[(
            "users",
            serde_json::json!([
                {"username": "a", "databases": ["d1", "d2"]},
                {"username": "b", "databases": ["d3"]},
            ]),
        )]);
        let v = engine()
            .render_str("{{ users | subelements('databases') }}", &s)
            .unwrap();
        let pairs: Vec<Value> = v.try_iter().unwrap().collect();
        assert_eq!(pairs.len(), 3);
        let first = pairs[0].get_item(&Value::from(0)).unwrap();
        assert_eq!(first.get_attr("username").unwrap().as_str(), Some("a"));
        assert_eq!(
            pairs[0].get_item(&Value::from(1)).unwrap().as_str(),
            Some("d1")
        );
    }

    #[test]
    fn subelements_missing_key_is_error() {
        let s = scope(&[("users", serde_json::json!([{"username": "a"}]))]);
        assert!(
            engine()
                .render_str("{{ users | subelements('databases') }}", &s)
                .is_err()
        );
    }

    #[test]
    fn lookup_dry_secrets_deterministic_and_memoized() {
        let e = engine();
        let s = Scope::new();
        let t = "{{ lookup('community.general.onepassword', 'titan SSH', field='private key', vault='ChainArgos') }}";
        let a = e.render_str(t, &s).unwrap();
        let b = e.render_str(t, &s).unwrap();
        assert_eq!(a.as_str(), b.as_str());
        assert!(a.as_str().unwrap().starts_with("dry-secret-"));
    }

    #[test]
    fn unknown_lookup_plugin_is_error() {
        let e = engine();
        let err = e
            .render_str("{{ lookup('file', '/etc/passwd') }}", &Scope::new())
            .unwrap_err();
        assert!(err.to_string().contains("outside the closed surface"));
    }

    #[test]
    fn undefined_template_result_is_error() {
        // Oracle: AnsibleUndefinedVariable (pinned 2026-06-11).
        assert!(matches!(
            engine().render_str("{{ nope }}", &Scope::new()),
            Err(EngineError::Undefined(_))
        ));
        assert!(matches!(
            engine().eval_condition(&Condition::Expr("nope".into()), &Scope::new()),
            Err(EngineError::Undefined(_))
        ));
    }

    #[test]
    fn concat_of_two_expressions_is_a_string() {
        // Oracle: '12' (string), not 12 — 2.21 does not literal_eval concat
        // results (pinned 2026-06-11).
        let s = scope(&[("a", serde_json::json!(1)), ("b", serde_json::json!(2))]);
        let v = engine().render_str("{{ a }}{{ b }}", &s).unwrap();
        assert_eq!(v.as_str(), Some("12"));
    }

    #[test]
    fn space_wrapped_expression_is_a_string() {
        // Oracle: ' 1 ' — whitespace outside delimiters defeats native mode.
        let s = scope(&[("a", serde_json::json!(1))]);
        let v = engine().render_str(" {{ a }} ", &s).unwrap();
        assert_eq!(v.as_str(), Some(" 1 "));
    }

    #[test]
    fn urlencode_matches_python_quote() {
        // Oracle: 'a%20b/c%2Bd%26e%3Df' — space %20, / kept, + encoded.
        let s = scope(&[("pw", serde_json::json!("a b/c+d&e=f"))]);
        let v = engine().render_str("{{ pw | urlencode }}", &s).unwrap();
        assert_eq!(v.as_str(), Some("a%20b/c%2Bd%26e%3Df"));
    }

    #[test]
    fn variable_cycle_is_contained() {
        let s = raw_scope("a: \"{{ b }}\"\nb: \"{{ a }}\"\n");
        let v = engine().render_str("{{ a }}", &s).unwrap();
        assert!(v.as_str().unwrap().contains("cycle"));
    }

    #[test]
    fn length_comparison_filter_precedence() {
        let s = scope(&[("tok", serde_json::json!("ghp_0123456789abcdef0123"))]);
        assert!(
            engine()
                .eval_condition(&Condition::Expr("tok | length >= 20".into()), &s)
                .unwrap()
        );
    }

    #[test]
    fn template_file_render_keeps_trailing_newline() {
        let s = scope(&[("x", serde_json::json!("v"))]);
        let out = engine().render_template_file("key={{ x }}\n", &s).unwrap();
        assert_eq!(out, "key=v\n");
    }

    #[test]
    fn render_value_recurses_containers() {
        let yaml: Yaml =
            serde_norway::from_str("pvs:\n  - \"{{ disk }}\"\n  - /dev/sdb\nname: data\n").unwrap();
        let s = scope(&[("disk", serde_json::json!("/dev/sda"))]);
        let v = engine().render_value(&yaml, &s).unwrap();
        let pvs = v.get_attr("pvs").unwrap();
        assert_eq!(
            pvs.get_item(&Value::from(0)).unwrap().as_str(),
            Some("/dev/sda")
        );
    }
}
