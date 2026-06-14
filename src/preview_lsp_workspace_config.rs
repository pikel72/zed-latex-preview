use zed_extension_api::serde_json::{json, Value};

pub fn get(settings: Value) -> Option<Value> {
    let preamble = match settings.get("preamble") {
        Some(Value::String(s)) => json!([s.as_str()]),
        Some(Value::Array(arr)) => Value::Array(
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                .collect(),
        ),
        _ => json!([]),
    };

    Some(json!({
        "enabled": settings.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        "maxFormulaLength": settings.get("maxFormulaLength").and_then(|v| v.as_u64()).unwrap_or(2000),
        "timeoutMs": settings.get("timeoutMs").and_then(|v| v.as_u64()).unwrap_or(1500),
        "scale": settings.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.4),
        "color": settings.get("color").and_then(|v| v.as_str()).unwrap_or("auto"),
        "renderer": "mathjax",
        "preamble": preamble,
    }))
}
