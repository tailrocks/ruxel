use std::{collections::BTreeMap, fmt};
use zeroize::Zeroizing;

pub struct Secret {
    name: String,
    value: Zeroizing<String>,
}

impl Secret {
    #[must_use]
    pub fn new(name: impl Into<String>, value: String) -> Self {
        Self {
            name: name.into(),
            value: Zeroizing::new(value),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        self.value.as_str()
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self::new(self.name.clone(), self.expose().to_owned())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<secret {}>", self.name)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<secret {}>", self.name)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SecretMap(pub BTreeMap<String, Secret>);

impl SecretMap {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Secret> {
        self.0.get(name)
    }
}

#[cfg(test)]
#[path = "secret/tests.rs"]
mod tests;
