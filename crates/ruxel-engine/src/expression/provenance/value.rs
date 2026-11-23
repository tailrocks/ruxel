use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Provenance {
    #[default]
    Public,
    Secret,
    Array(Vec<Provenance>),
    Object(BTreeMap<String, Provenance>),
}

impl Provenance {
    #[must_use]
    pub fn is_secret(&self) -> bool {
        match self {
            Self::Public => false,
            Self::Secret => true,
            Self::Array(items) => items.iter().any(Self::is_secret),
            Self::Object(items) => items.values().any(Self::is_secret),
        }
    }

    pub(super) fn validated(&self, value: &Value) -> Self {
        match (self, value) {
            (Self::Public, _) => Self::Public,
            (Self::Array(provenance), Value::Array(values)) if provenance.len() == values.len() => {
                Self::Array(
                    provenance
                        .iter()
                        .zip(values)
                        .map(|(p, v)| p.validated(v))
                        .collect(),
                )
            }
            (Self::Object(provenance), Value::Object(values))
                if provenance.keys().all(|k| values.contains_key(k)) =>
            {
                Self::Object(
                    provenance
                        .iter()
                        .filter_map(|(k, p)| values.get(k).map(|v| (k.clone(), p.validated(v))))
                        .collect(),
                )
            }
            _ => Self::Secret,
        }
    }

    pub(super) fn child(&self, key: &str) -> Self {
        match self {
            Self::Public => Self::Public,
            Self::Secret => Self::Secret,
            Self::Array(items) => key
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .cloned()
                .unwrap_or(Self::Secret),
            Self::Object(items) => items.get(key).cloned().unwrap_or_default(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct TrackedValue {
    pub value: Value,
    pub provenance: Provenance,
}

impl std::fmt::Debug for TrackedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackedValue")
            .field("value", &self.redacted())
            .field("provenance", &self.provenance)
            .finish()
    }
}

impl TrackedValue {
    #[must_use]
    pub fn public(value: Value) -> Self {
        Self {
            value,
            provenance: Provenance::Public,
        }
    }
    #[must_use]
    pub fn secret(value: Value) -> Self {
        Self {
            value,
            provenance: Provenance::Secret,
        }
    }
    #[must_use]
    pub fn is_secret(&self) -> bool {
        self.provenance.validated(&self.value).is_secret()
    }

    #[must_use]
    pub fn object(fields: BTreeMap<String, Self>) -> Self {
        Self {
            value: Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.value.clone()))
                    .collect(),
            ),
            provenance: Provenance::Object(
                fields.into_iter().map(|(k, v)| (k, v.provenance)).collect(),
            ),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<Self> {
        let value = match &self.value {
            Value::Object(fields) => fields.get(key),
            Value::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
            _ => None,
        }?;
        Some(Self {
            value: value.clone(),
            provenance: self.provenance.validated(&self.value).child(key),
        })
    }

    /// # Errors
    /// Returns an error when this value or its provenance is not an object.
    pub fn insert(&mut self, key: String, value: Self) -> Result<()> {
        let fields = self
            .value
            .as_object_mut()
            .context("tracked value must be an object")?;
        match &mut self.provenance {
            Provenance::Public => {
                self.provenance =
                    Provenance::Object(BTreeMap::from([(key.clone(), value.provenance)]));
            }
            Provenance::Secret => {}
            Provenance::Object(tree) => {
                tree.insert(key.clone(), value.provenance);
            }
            Provenance::Array(_) => bail!("object has incompatible provenance"),
        }
        fields.insert(key, value.value);
        Ok(())
    }

    #[must_use]
    pub fn redacted(&self) -> Value {
        fn redact(value: &Value, provenance: &Provenance) -> Value {
            match (value, provenance) {
                (Value::Array(values), Provenance::Array(items)) if values.len() == items.len() => {
                    Value::Array(
                        values
                            .iter()
                            .zip(items)
                            .map(|(v, p)| redact(v, p))
                            .collect(),
                    )
                }
                (Value::Object(values), Provenance::Object(items)) => Value::Object(
                    values
                        .iter()
                        .map(|(key, v)| {
                            (
                                key.clone(),
                                redact(v, items.get(key).unwrap_or(&Provenance::Public)),
                            )
                        })
                        .collect(),
                ),
                (_, Provenance::Public) => value.clone(),
                _ => Value::String("[REDACTED]".into()),
            }
        }
        redact(&self.value, &self.provenance.validated(&self.value))
    }
}
