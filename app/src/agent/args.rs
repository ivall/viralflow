//! Reading tool arguments. Models write them in many slightly different ways
//! (camelCase or snake_case keys, numbers as text, one object where a list
//! belongs), so reading is forgiving, but a value that cannot mean anything
//! is an error that says what was expected.

use serde_json::{Map, Value};

pub type Args = Map<String, Value>;

/// `fadeIn`, `fade-in` and `fade_in` are all `fade_in`.
pub fn snake(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for (index, c) in key.chars().enumerate() {
        if c.is_uppercase() {
            if index > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else if c == '-' || c == ' ' {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    out
}

/// The arguments as an object with snake_case keys. Models sometimes send the
/// object as JSON text.
pub fn object(value: &Value) -> Result<Args, String> {
    match value {
        Value::Object(map) => Ok(map.iter().map(|(k, v)| (snake(k), v.clone())).collect()),
        Value::Null => Ok(Args::new()),
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(inner @ Value::Object(_)) => object(&inner),
            _ => Err("The arguments must be a JSON object.".into()),
        },
        _ => Err("The arguments must be a JSON object.".into()),
    }
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().trim_end_matches('s').trim_end_matches('%').trim().parse().ok(),
        _ => None,
    }
    .filter(|v: &f64| v.is_finite())
}

pub fn number(args: &Args, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => as_number(value).map(Some).ok_or_else(|| format!("`{key}` must be a number, not {value}")),
    }
}

pub fn required_number(args: &Args, key: &str) -> Result<f64, String> {
    number(args, key)?.ok_or_else(|| format!("`{key}` is required"))
}

pub fn string(args: &Args, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(Value::Bool(b)) => Ok(Some(b.to_string())),
        Some(value) => Err(format!("`{key}` must be text, not {value}")),
    }
}

pub fn required_string(args: &Args, key: &str) -> Result<String, String> {
    string(args, key)?.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("`{key}` is required"))
}

pub fn boolean(args: &Args, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("true") => Ok(Some(true)),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("false") => Ok(Some(false)),
        Some(Value::Number(n)) => Ok(Some(n.as_f64() != Some(0.0))),
        Some(value) => Err(format!("`{key}` must be true or false, not {value}")),
    }
}

/// A list of ids (one id alone is a list of one).
pub fn strings(args: &Args, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.clone()),
                Value::Object(map) => map.get("id").and_then(Value::as_str).map(String::from).ok_or_else(|| format!("`{key}` must list ids")),
                other => Err(format!("`{key}` must list ids, not {other}")),
            })
            .collect(),
        Some(value) => Err(format!("`{key}` must be a list of ids, not {value}")),
    }
}

/// A list of objects (one object alone is a list of one).
pub fn objects(args: &Args, key: &str) -> Result<Vec<Args>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(items)) => items.iter().map(object).collect(),
        Some(value @ Value::Object(_)) => Ok(vec![object(value)?]),
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(items)) => items.iter().map(object).collect(),
            Ok(inner @ Value::Object(_)) => Ok(vec![object(&inner)?]),
            _ => Err(format!("`{key}` must be a list of objects")),
        },
        Some(value) => Err(format!("`{key}` must be a list of objects, not {value}")),
    }
}

/// A list of numbers (one number alone is a list of one).
pub fn numbers(args: &Args, key: &str) -> Result<Vec<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(items)) => items.iter().map(|v| as_number(v).ok_or_else(|| format!("`{key}` must list numbers, not {v}"))).collect(),
        Some(value) => as_number(value).map(|v| vec![v]).ok_or_else(|| format!("`{key}` must be a list of numbers, not {value}")),
    }
}

/// A value of one of the app's named kinds (effects, fonts…), with the
/// choices listed when it is not one of them.
pub fn choice<T: Copy>(args: &Args, key: &str, parse: impl Fn(&str) -> Option<T>, names: &[&str]) -> Result<Option<T>, String> {
    let Some(text) = string(args, key)? else { return Ok(None) };
    let wanted = text.trim().to_lowercase().replace([' ', '_'], "-");
    parse(&wanted).map(Some).ok_or_else(|| format!("`{key}` \"{text}\" is not one of: {}", names.join(", ")))
}

/// Keys of `args` that are not in `known` (reported instead of ignored, so a
/// model learns the right name).
pub fn unknown_keys(args: &Args, known: &[&str]) -> Option<String> {
    let unknown: Vec<&str> = args.keys().map(String::as_str).filter(|k| !known.contains(k)).collect();
    (!unknown.is_empty()).then(|| format!("Unknown field(s): {}. Known fields: {}.", unknown.join(", "), known.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_and_values_are_read_forgivingly() {
        assert_eq!(snake("fadeIn"), "fade_in");
        assert_eq!(snake("letter-spacing"), "letter_spacing");
        assert_eq!(snake("x"), "x");
        let args = object(&json!({ "fadeIn": "0.5s", "itemIds": "a", "flip": "true", "times": 2 })).unwrap();
        assert_eq!(number(&args, "fade_in").unwrap(), Some(0.5));
        assert_eq!(strings(&args, "item_ids").unwrap(), ["a"]);
        assert_eq!(boolean(&args, "flip").unwrap(), Some(true));
        assert_eq!(numbers(&args, "times").unwrap(), [2.0]);
        assert!(number(&object(&json!({ "x": "left" })).unwrap(), "x").is_err());
        let wrapped = object(&json!("{\"a\": 1}")).unwrap();
        assert_eq!(number(&wrapped, "a").unwrap(), Some(1.0));
        assert_eq!(objects(&object(&json!({ "texts": { "content": "hi" } })).unwrap(), "texts").unwrap().len(), 1);
        assert!(unknown_keys(&args, &["fade_in"]).unwrap().contains("item_ids"));
    }
}
