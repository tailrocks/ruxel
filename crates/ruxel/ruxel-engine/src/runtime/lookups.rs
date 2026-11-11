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
    /// The exact resolved values; the redactor derives their encoded forms.
    pub(super) values: Mutex<Vec<String>>,
    /// `values` plus every derived form, rebuilt when a value is recorded.
    patterns: Mutex<Vec<String>>,
}
impl Lookups {
    pub(super) fn new(resolver: Resolver) -> Self {
        Self::with_values(resolver, Vec::new())
    }

    /// Starts from already-resolved values, with their derived forms.
    pub(super) fn with_values(resolver: Resolver, values: Vec<String>) -> Self {
        let patterns = derived_patterns(&values);
        Self {
            resolver,
            values: Mutex::new(values),
            patterns: Mutex::new(patterns),
        }
    }

    /// Records one resolved secret for the redactor, together with its
    /// derived forms: the exact value, its standard and URL-safe base64 and
    /// its hex encodings. An interpolated or joined secret carries the exact
    /// value as a substring, so the value's own patterns cover those
    /// compositions; the encodings cover a secret that passes through a
    /// base64 or hex step in the workflow's expressions.
    fn record(&self, value: &str) {
        let Ok(mut values) = self.values.lock() else {
            return;
        };
        values.push(value.to_owned());
        let derived = derived_patterns(&values);
        drop(values);
        if let Ok(mut patterns) = self.patterns.lock() {
            *patterns = derived;
        }
    }

    /// Records a value through the same path resolution does; the no-
    /// plaintext grep test drives this directly.
    #[cfg(test)]
    pub(super) fn record_value_for_tests(&self, value: &str) {
        self.record(value);
    }

    fn patterns(&self) -> Vec<String> {
        self.patterns
            .lock()
            .map(|patterns| patterns.clone())
            .unwrap_or_default()
    }

    /// The keyed fingerprint a changed secret-bearing file shows in check
    /// and diff: a keyed, truncated digest of the selector-derived
    /// placeholders of the lookups in the file's content — never text, and
    /// never an unsalted hash of a value. The placeholder derives from the
    /// selector, so the sentence is rotation-blind by design, which the
    /// design states rather than hides.
    pub(super) fn content_fingerprint(&self, params: &Value) -> Option<String> {
        let content = params.get("content")?;
        let mut lookups = Vec::new();
        collect_lookups(content, &mut lookups);
        if lookups.is_empty() {
            return None;
        }
        let mut placeholders = Vec::with_capacity(lookups.len());
        for expr in &lookups {
            placeholders.push(crate::secrets::selector_placeholder(
                &Self::lookup(expr).ok()?,
            ));
        }
        Some(
            self.resolver
                .fingerprint("content", &placeholders.join("\u{1f}")),
        )
    }

    pub(super) fn lookup(expr: &Expr) -> Result<Lookup> {
        let Expr::Lookup {
            provider,
            key,
            format,
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
            "fnox-generated" => Lookup::FnoxGenerated {
                key: required(key)?,
                format: required(format)?,
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
    pub(super) fn redact(&self, value: &Value) -> Value {
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
        let patterns = self.patterns();
        scrub(value, &patterns)
    }
}
impl LookupResolver for Lookups {
    fn resolve(&self, expr: &Expr) -> Result<Value> {
        let value = self
            .resolver
            .resolve(&Self::lookup(expr)?)
            .map_err(anyhow::Error::msg)?;
        self.record(&value);
        Ok(Value::String(value))
    }
}

#[cfg(test)]
mod tests;

/// Every pattern the redactor matches: the exact resolved values and their
/// standard and URL-safe base64 and hex forms.
fn derived_patterns(values: &[String]) -> Vec<String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let base64 = |urlsafe: bool, text: &[u8]| -> String {
        let mut out = String::new();
        for chunk in text.chunks(3) {
            let bytes = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
            out.push(char::from(TABLE[(n >> 18) as usize & 63]));
            out.push(char::from(TABLE[(n >> 12) as usize & 63]));
            out.push(if chunk.len() > 1 {
                char::from(TABLE[(n >> 6) as usize & 63])
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                char::from(TABLE[n as usize & 63])
            } else {
                '='
            });
        }
        if urlsafe {
            out.replace('+', "-").replace('/', "_")
        } else {
            out
        }
    };
    let mut patterns = Vec::new();
    for value in values {
        patterns.push(value.clone());
        let bytes = value.as_bytes();
        for encoded in [base64(false, bytes), base64(true, bytes)] {
            patterns.push(encoded.trim_end_matches('=').to_owned());
            patterns.push(encoded);
        }
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            hex.push(char::from(HEX[usize::from(byte >> 4)]));
            hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        patterns.push(hex);
    }
    patterns.sort();
    patterns.dedup();
    patterns
}

/// Every lookup expression of a value, in field order.
fn collect_lookups(value: &Value, out: &mut Vec<Expr>) {
    match value {
        Value::Object(fields) => {
            if fields.get("kind").and_then(Value::as_str) == Some("lookup")
                && let Ok(expr) = serde_json::from_value::<Expr>(value.clone())
            {
                out.push(expr);
                return;
            }
            for child in fields.values() {
                collect_lookups(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_lookups(child, out);
            }
        }
        _ => {}
    }
}
