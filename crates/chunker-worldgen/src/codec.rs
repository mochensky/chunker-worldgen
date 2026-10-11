//! JSON helpers matching DataFixerUpper `JsonOps` number semantics.
//!
//! Gson keeps numbers as decimal strings (`LazilyParsedNumber`), so `Codec.FLOAT` is
//! `Float.parseFloat(text)` — *not* `(float) Double.parseDouble(text)`. serde_json is built with
//! `arbitrary_precision`, which lets us parse the original text the same way.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

pub fn as_f32(v: &Value) -> Result<f32> {
    match v {
        Value::Number(n) => n
            .to_string()
            .parse::<f32>()
            .map_err(|e| anyhow!("bad float {n}: {e}")),
        Value::Bool(b) => Ok(*b as i32 as f32),
        _ => bail!("expected number, got {v}"),
    }
}

pub fn as_f64(v: &Value) -> Result<f64> {
    match v {
        Value::Number(n) => n
            .to_string()
            .parse::<f64>()
            .map_err(|e| anyhow!("bad double {n}: {e}")),
        Value::Bool(b) => Ok(*b as i32 as f64),
        _ => bail!("expected number, got {v}"),
    }
}

pub fn as_i32(v: &Value) -> Result<i32> {
    match v {
        Value::Number(n) => {
            let text = n.to_string();
            if let Ok(i) = text.parse::<i32>() {
                return Ok(i);
            }
            if let Ok(l) = text.parse::<i64>() {
                return Ok(l as i32);
            }
            Ok(text.parse::<f64>().map_err(|e| anyhow!("bad int {text}: {e}"))? as i32)
        }
        Value::Bool(b) => Ok(*b as i32),
        _ => bail!("expected integer, got {v}"),
    }
}

pub fn as_i64(v: &Value) -> Result<i64> {
    match v {
        Value::Number(n) => {
            let text = n.to_string();
            if let Ok(l) = text.parse::<i64>() {
                return Ok(l);
            }
            Ok(text.parse::<f64>().map_err(|e| anyhow!("bad long {text}: {e}"))? as i64)
        }
        _ => bail!("expected integer, got {v}"),
    }
}

pub fn as_bool(v: &Value) -> Result<bool> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => Ok(n.to_string().parse::<f64>().unwrap_or(0.0) != 0.0),
        _ => bail!("expected boolean, got {v}"),
    }
}

pub fn as_str(v: &Value) -> Result<&str> {
    v.as_str().with_context(|| format!("expected string, got {v}"))
}

pub fn field<'a>(obj: &'a Value, name: &str) -> Result<&'a Value> {
    obj.get(name).with_context(|| format!("missing field `{name}`"))
}

pub fn f32_field(obj: &Value, name: &str) -> Result<f32> {
    as_f32(field(obj, name)?).with_context(|| format!("field `{name}`"))
}

pub fn f64_field(obj: &Value, name: &str) -> Result<f64> {
    as_f64(field(obj, name)?).with_context(|| format!("field `{name}`"))
}

pub fn i32_field(obj: &Value, name: &str) -> Result<i32> {
    as_i32(field(obj, name)?).with_context(|| format!("field `{name}`"))
}

pub fn str_field<'a>(obj: &'a Value, name: &str) -> Result<&'a str> {
    as_str(field(obj, name)?).with_context(|| format!("field `{name}`"))
}

pub fn opt_f32(obj: &Value, name: &str, default: f32) -> Result<f32> {
    obj.get(name)
        .map(as_f32)
        .transpose()
        .map(|v| v.unwrap_or(default))
}

pub fn opt_f64(obj: &Value, name: &str, default: f64) -> Result<f64> {
    obj.get(name)
        .map(as_f64)
        .transpose()
        .map(|v| v.unwrap_or(default))
}

pub fn opt_i32(obj: &Value, name: &str, default: i32) -> Result<i32> {
    obj.get(name)
        .map(as_i32)
        .transpose()
        .map(|v| v.unwrap_or(default))
}

pub fn opt_bool(obj: &Value, name: &str, default: bool) -> Result<bool> {
    obj.get(name)
        .map(as_bool)
        .transpose()
        .map(|v| v.unwrap_or(default))
}

/// Strips the `minecraft:` namespace from a registry type id (`"minecraft:noise"` -> `"noise"`).
pub fn type_name(obj: &Value) -> Result<String> {
    let t = str_field(obj, "type")?;
    Ok(match t.split_once(':') {
        Some(("minecraft", rest)) => rest.to_owned(),
        Some((ns, rest)) => format!("{ns}:{rest}"),
        None => t.to_owned(),
    })
}

pub fn as_array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array()
        .with_context(|| format!("expected an array, got {v}"))
}
