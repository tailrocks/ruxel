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
    #[error("undefined variable in {0:?}")]
    Undefined(String),
    #[error("variable cycle while rendering {0:?}")]
    VarCycle(String),
    #[error("failed to render variable {0:?}: {1}")]
    LazyVar(String, String),
    #[error("unsupported YAML value (tagged) in template scope")]
    TaggedValue,
}

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

fn python_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None".into(),
        serde_json::Value::Bool(value) => if *value { "True" } else { "False" }.into(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_display(&key.clone().into()),
                    python_display(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

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

#[derive(Clone)]
pub enum VarValue {
    Raw(Yaml),
    Final(Value),
}

impl std::fmt::Debug for VarValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw(_) => f.write_str("Raw(<redacted>)"),
            Self::Final(_) => f.write_str("Final(<redacted>)"),
        }
    }
}

#[derive(Clone, Default)]
pub struct Scope {
    layers: Vec<Arc<Vec<(String, VarValue)>>>,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field(
                "layers",
                &format_args!("<redacted; {} layers>", self.layers.len()),
            )
            .finish()
    }
}

impl Scope {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
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

struct ScopeObject {
    engine: Arc<EngineInner>,
    scope: Scope,
    memo: Mutex<HashMap<String, Value>>,
    in_flight: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for ScopeObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopeObject")
            .field("engine", &"<redacted>")
            .field("scope", &"<redacted>")
            .field("memo", &"<redacted>")
            .field("in_flight", &"<redacted>")
            .finish()
    }
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
                    return Some(Value::from_object(ScopeRenderError {
                        name: name.to_string(),
                        message: "variable cycle".into(),
                        cycle: true,
                    }));
                }
                let result = render_yaml(&self.engine, &yaml, self);
                self.in_flight.lock().unwrap().remove(name);
                match result {
                    Ok(v) => v,
                    Err(e) => {
                        let cycle = matches!(e, EngineError::VarCycle(_));
                        return Some(Value::from_object(ScopeRenderError {
                            name: name.to_string(),
                            message: e.to_string(),
                            cycle,
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

#[derive(Debug)]
struct ScopeRenderError {
    name: String,
    message: String,
    cycle: bool,
}

impl Object for ScopeRenderError {
    fn get_value(self: &Arc<Self>, _key: &Value) -> Option<Value> {
        None
    }

    fn render(self: &Arc<Self>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    where
        Self: Sized + 'static,
    {
        let _ = f;
        Err(std::fmt::Error)
    }
}

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
        env.set_undefined_behavior(UndefinedBehavior::Chainable);
        env.set_formatter(|out, _state, value| {
            if value.is_undefined() {
                return Err(minijinja::Error::new(
                    MjErrorKind::UndefinedError,
                    "undefined value in template output",
                ));
            }
            match value.kind() {
                minijinja::value::ValueKind::Bool => {
                    write!(out, "{}", if value.is_true() { "True" } else { "False" })
                }
                minijinja::value::ValueKind::None => write!(out, "None"),
                minijinja::value::ValueKind::Seq => {
                    let json = serde_json::to_value(value).map_err(|error| {
                        minijinja::Error::new(MjErrorKind::InvalidOperation, error.to_string())
                    })?;
                    write!(out, "{}", python_display(&json))
                }
                minijinja::value::ValueKind::Map => {
                    let json = serde_json::to_value(value).map_err(|error| {
                        minijinja::Error::new(MjErrorKind::InvalidOperation, error.to_string())
                    })?;
                    if json.as_object().is_some_and(|map| !map.is_empty()) {
                        write!(out, "{}", python_display(&json))
                    } else {
                        write!(out, "{value}")
                    }
                }
                _ => write!(out, "{value}"),
            }
            .map_err(minijinja::Error::from)
        });
        env.set_keep_trailing_newline(true);
        env.set_trim_blocks(true);

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

    pub fn render_str(&self, template: &str, scope: &Scope) -> Result<Value, EngineError> {
        render_str_inner(&self.inner, template, &self.context(scope))
    }

    pub fn render_template_file(
        &self,
        content: &str,
        scope: &Scope,
    ) -> Result<String, EngineError> {
        Ok(self.inner.env.render_str(content, self.context(scope))?)
    }

    pub fn render_value(&self, value: &Yaml, scope: &Scope) -> Result<Value, EngineError> {
        let obj = self.scope_object(scope);
        render_yaml(&self.inner, value, &obj)
    }

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
    if value.is_undefined() {
        return Err(EngineError::Undefined(expr.to_string()));
    }
    if let Some(error) = poison_error(&value) {
        return Err(error);
    }
    Ok(value.is_true())
}

fn poison_error(value: &Value) -> Option<EngineError> {
    let error = value.downcast_object_ref::<ScopeRenderError>()?;
    Some(if error.cycle {
        EngineError::VarCycle(error.name.clone())
    } else {
        EngineError::LazyVar(error.name.clone(), error.message.clone())
    })
}

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
        if value.is_undefined() {
            return Err(EngineError::Undefined(template.to_string()));
        }
        if let Some(error) = poison_error(&value) {
            return Err(error);
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingResolver(Arc<AtomicUsize>);

    impl LookupResolver for CountingResolver {
        fn onepassword(
            &self,
            item: &str,
            _field: Option<&str>,
            _vault: Option<&str>,
            _section: Option<&str>,
        ) -> Result<String, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(item.to_string())
        }

        fn pipe(&self, cmd: &str) -> Result<String, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(cmd.to_string())
        }
    }

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
    fn scope_debug_redacts_values() {
        let marker = "synthetic-debug-marker";
        let scope = scope(&[("credential", serde_json::json!(marker))]);
        let object = engine().scope_object(&scope);
        object
            .memo
            .lock()
            .unwrap()
            .insert("credential".into(), Value::from(marker));

        assert!(!format!("{scope:?}").contains(marker));
        assert!(!format!("{object:?}").contains(marker));
        assert!(!format!("{:?}", VarValue::Final(Value::from(marker))).contains(marker));
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
        let calls = Arc::new(AtomicUsize::new(0));
        let e = Engine::new(Arc::new(MemoizedResolver::new(CountingResolver(
            calls.clone(),
        ))));
        let s = Scope::new();
        let t = "{{ lookup('community.general.onepassword', 'fixture_10 SSH', field='private key', vault='exampleorg') }}";
        let a = e.render_str(t, &s).unwrap();
        let b = e.render_str(t, &s).unwrap();
        assert_eq!(a.as_str(), b.as_str());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        e.render_str("{{ lookup('pipe', 'printf distinct') }}", &s)
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
        let s = scope(&[("a", serde_json::json!(1)), ("b", serde_json::json!(2))]);
        let v = engine().render_str("{{ a }}{{ b }}", &s).unwrap();
        assert_eq!(v.as_str(), Some("12"));
    }

    #[test]
    fn space_wrapped_expression_is_a_string() {
        let s = scope(&[("a", serde_json::json!(1))]);
        let v = engine().render_str(" {{ a }} ", &s).unwrap();
        assert_eq!(v.as_str(), Some(" 1 "));
    }

    #[test]
    fn urlencode_matches_python_quote() {
        let s = scope(&[("pw", serde_json::json!("a b/c+d&e=f"))]);
        let v = engine().render_str("{{ pw | urlencode }}", &s).unwrap();
        assert_eq!(v.as_str(), Some("a%20b/c%2Bd%26e%3Df"));
    }

    #[test]
    fn variable_cycle_is_a_hard_error() {
        let s = raw_scope("a: \"{{ b }}\"\nb: \"{{ a }}\"\n");
        let result = engine().render_str("{{ a }}", &s);
        assert!(
            matches!(result, Err(EngineError::VarCycle(_))),
            "unexpected cycle result: {result:?}"
        );
    }

    #[test]
    fn failed_lazy_variable_is_not_truthy_or_rendered() {
        let s = raw_scope("bad: \"{{ missing }}\"\n");
        assert!(matches!(
            engine().eval_condition(&Condition::Expr("bad".into()), &s),
            Err(EngineError::LazyVar(_, _))
        ));
        assert!(
            engine()
                .render_template_file("value={{ bad }}", &s)
                .is_err()
        );
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
    fn template_file_matches_ansible_block_trimming() {
        let s = scope(&[("items", serde_json::json!(["a", "b"]))]);
        let out = engine()
            .render_template_file("{% for item in items %}\n{{ item }}\n{% endfor %}\n", &s)
            .unwrap();
        assert_eq!(out, "a\nb\n");
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
