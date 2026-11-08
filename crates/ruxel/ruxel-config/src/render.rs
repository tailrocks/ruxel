use serde_json::Value;

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Yaml,
    Toml,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("toml: the root must be an object")]
    TomlRoot,
    #[error("toml: {0}")]
    Toml(String),
    #[error("json: {0}")]
    Json(String),
}

/// Renders a JSON value in the requested output format.
///
/// # Errors
///
/// Returns a format-specific error when TOML cannot represent the value, its
/// root is not an object, or JSON serialization fails.
pub fn render(format: Format, value: &Value) -> Result<String, RenderError> {
    match format {
        Format::Json => pretty_json(value).map(|rendered| format!("{rendered}\n")),
        Format::Yaml => {
            if let Value::Array(documents) = value {
                let rendered = documents
                    .iter()
                    .map(pretty_json)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("{}\n", rendered.join("\n---\n")))
            } else {
                pretty_json(value).map(|rendered| format!("{rendered}\n"))
            }
        }
        Format::Toml => {
            let Value::Object(_) = value else {
                return Err(RenderError::TomlRoot);
            };
            let mut value = value.clone();
            drop_nulls(&mut value)?;
            toml::to_string(&value).map_err(|error| RenderError::Toml(error.to_string()))
        }
    }
}

fn pretty_json(value: &Value) -> Result<String, RenderError> {
    serde_json::to_string_pretty(value).map_err(|error| RenderError::Json(error.to_string()))
}

fn drop_nulls(value: &mut Value) -> Result<(), RenderError> {
    match value {
        Value::Object(object) => {
            object.retain(|_, value| !value.is_null());
            for value in object.values_mut() {
                drop_nulls(value)?;
            }
        }
        Value::Array(array) => {
            for value in array {
                if value.is_null() {
                    return Err(RenderError::Toml(
                        "null array elements are not supported".into(),
                    ));
                }
                drop_nulls(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
