use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

pub(super) fn compare(left: &Value, right: &Value) -> Result<std::cmp::Ordering> {
    match (left, right) {
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b)),
        (Value::Number(a), Value::Number(b)) => {
            let integer = |n: &serde_json::Number| {
                n.as_i64()
                    .map(i128::from)
                    .or_else(|| n.as_u64().map(i128::from))
            };
            match (integer(a), integer(b)) {
                (Some(a), Some(b)) => Ok(a.cmp(&b)),
                (Some(a), None) => compare_int_float(a, b.as_f64().context("invalid number")?),
                (None, Some(b)) => compare_int_float(b, a.as_f64().context("invalid number")?)
                    .map(std::cmp::Ordering::reverse),
                (None, None) => a
                    .as_f64()
                    .zip(b.as_f64())
                    .and_then(|(a, b)| a.partial_cmp(&b))
                    .context("numbers cannot be compared"),
            }
        }
        _ => bail!("comparison expects two strings or two numbers"),
    }
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "the fractional part is compared separately after range checks"
)]
fn compare_int_float(integer: i128, float: f64) -> Result<std::cmp::Ordering> {
    use std::cmp::Ordering;
    ensure!(float.is_finite(), "numbers cannot be compared");
    if float >= 18_446_744_073_709_551_616.0 {
        return Ok(Ordering::Less);
    }
    if float <= -18_446_744_073_709_551_616.0 {
        return Ok(Ordering::Greater);
    }
    let truncated = float.trunc() as i128;
    Ok(integer.cmp(&truncated).then_with(|| {
        if float.fract() > 0.0 {
            Ordering::Less
        } else if float.fract() < 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }))
}
pub(super) fn equal(left: &Value, right: &Value) -> Result<bool> {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => Ok(compare(left, right)?.is_eq()),
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b) {
                if !equal(a, b)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Object(a), Value::Object(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (key, value) in a {
                match b.get(key) {
                    Some(other) if equal(value, other)? => {}
                    _ => return Ok(false),
                }
            }
            Ok(true)
        }
        _ => Ok(left == right),
    }
}
pub(super) fn render_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}
