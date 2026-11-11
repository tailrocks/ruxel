use crate::{
    expression::{Expr, LookupResolver},
    secrets::{Lookup, Resolver},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::sync::Mutex;

fn redact_text(text: &str, secrets: &[String]) -> String {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (start, _) in text.char_indices() {
        let end = secrets
            .iter()
            .filter(|secret| !secret.is_empty() && text[start..].starts_with(secret.as_str()))
            .map(|secret| start + secret.len())
            .max();
        if let Some(end) = end {
            if let Some(last) = spans.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                spans.push((start, end));
            }
        }
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in spans {
        output.push_str(&text[cursor..start]);
        output.push_str("[REDACTED]");
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

pub(super) struct Lookups {
    pub(super) resolver: Resolver,
    pub(super) values: Mutex<Vec<String>>,
}
impl Lookups {
    pub(super) fn lookup(expr: &Expr) -> Result<Lookup> {
        let Expr::Lookup {
            provider,
            key,
            vault,
            item,
            section,
            field,
            reference,
            argv,
        } = expr
        else {
            bail!("expected lookup");
        };
        let required = |value: &Option<String>| value.clone().context("missing lookup selector");
        Ok(match provider.as_str() {
            "fnox" => Lookup::Fnox {
                key: required(key)?,
            },
            "op" if reference.is_some() => {
                let reference = required(reference)?;
                if reference.is_empty() {
                    bail!("lookup reference must not be empty");
                }
                Lookup::OnePasswordUri { reference }
            }
            // No default vault or field: the structured selector states all
            // three, or the lookup fails rather than guessing a value.
            "op" => Lookup::OnePassword {
                vault: required(vault)?,
                item: required(item)?,
                field: required(field)?,
                section: section.clone(),
            },
            "command" => Lookup::Command { argv: argv.clone() },
            "synthetic" => Lookup::Synthetic {
                value: required(key)?,
            },
            _ => bail!("unknown lookup provider"),
        })
    }
    pub(super) fn redact(&self, value: &Value) -> Result<Value> {
        fn scrub(value: &Value, secrets: &[String]) -> Value {
            match value {
                Value::String(text) => Value::String(redact_text(text, secrets)),
                Value::Array(items) => {
                    Value::Array(items.iter().map(|v| scrub(v, secrets)).collect())
                }
                Value::Object(items) => Value::Object(
                    items
                        .iter()
                        .map(|(key, value)| {
                            let key = redact_text(key, secrets);
                            (key, scrub(value, secrets))
                        })
                        .collect(),
                ),
                _ => value.clone(),
            }
        }
        let secrets = self
            .values
            .lock()
            .map_err(|_| anyhow::anyhow!("redactor unavailable"))?;
        Ok(scrub(value, &secrets))
    }
}
impl LookupResolver for Lookups {
    fn resolve(&self, expr: &Expr) -> Result<Value> {
        let value = self
            .resolver
            .resolve(&Self::lookup(expr)?)
            .map_err(anyhow::Error::msg)?;
        self.values
            .lock()
            .map_err(|_| anyhow::anyhow!("redactor unavailable"))?
            .push(value.clone());
        Ok(Value::String(value))
    }
}

#[cfg(test)]
mod tests;
