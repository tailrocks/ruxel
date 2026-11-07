use crate::{
    expression::{Expr, LookupResolver},
    secrets::{Lookup, Resolver},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::sync::Mutex;

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
            "op" => Lookup::OnePassword {
                vault: vault.clone().unwrap_or_else(|| "Private".to_owned()),
                item: required(item)?,
                field: field.clone().unwrap_or_else(|| "password".to_owned()),
                section: section.clone(),
            },
            "command" => Lookup::Command { argv: argv.clone() },
            "pipe" => Lookup::Pipe {
                command: key
                    .clone()
                    .or_else(|| {
                        if argv.len() == 1 {
                            argv.first().cloned()
                        } else {
                            None
                        }
                    })
                    .context("pipe lookup requires a single command string")?,
            },
            "synthetic" => Lookup::Synthetic {
                value: required(key)?,
            },
            _ => bail!("unknown lookup provider"),
        })
    }
    pub(super) fn redact(&self, value: &Value) -> Result<Value> {
        fn scrub(value: &Value, secrets: &[String]) -> Value {
            match value {
                Value::String(text) => Value::String(
                    secrets
                        .iter()
                        .filter(|s| !s.is_empty())
                        .fold(text.clone(), |text, secret| {
                            text.replace(secret, "[REDACTED]")
                        }),
                ),
                Value::Array(items) => {
                    Value::Array(items.iter().map(|v| scrub(v, secrets)).collect())
                }
                Value::Object(items) => Value::Object(
                    items
                        .iter()
                        .map(|(key, value)| {
                            let key = secrets
                                .iter()
                                .filter(|s| !s.is_empty())
                                .fold(key.clone(), |key, secret| key.replace(secret, "[REDACTED]"));
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
