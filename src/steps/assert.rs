use crate::json::{matcher, path};
use crate::world::World;

fn last(w: &World) -> Result<&crate::http::Exchange, String> {
    w.http
        .last()
        .ok_or_else(|| "no request has been sent yet".to_string())
}

pub fn response_code(w: &World, expected: &str) -> Result<(), String> {
    let want: u16 = expected
        .parse()
        .map_err(|_| format!("invalid code {expected:?}"))?;
    let got = last(w)?.status;
    if got == want {
        Ok(())
    } else {
        Err(format!("    expected: {want}\n    actual:   {got}"))
    }
}

pub fn response_header(w: &World, name: &str, value: &str) -> Result<(), String> {
    let ex = last(w)?;
    let got = ex
        .resp_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str());
    match got {
        Some(v) if v == value => Ok(()),
        Some(v) => Err(format!("    expected: {value}\n    actual:   {v}")),
        None => Err(format!("response header {name:?} is missing")),
    }
}

pub fn body_contains_json(w: &World, docstring: Option<&String>) -> Result<(), String> {
    let expected_src = docstring.ok_or("step requires a doc string with JSON")?;
    let expected: serde_json::Value = serde_json::from_str(expected_src)
        .map_err(|e| format!("expected JSON is invalid: {e}"))?;
    let actual = last(w)?.json()?;
    matcher::contains(&actual, &expected).map_err(|m| m.to_string())
}

pub fn body_equals_json(w: &World, docstring: Option<&String>) -> Result<(), String> {
    let expected_src = docstring.ok_or("step requires a doc string with JSON")?;
    let expected: serde_json::Value = serde_json::from_str(expected_src)
        .map_err(|e| format!("expected JSON is invalid: {e}"))?;
    let actual = last(w)?.json()?;
    matcher::equals(&actual, &expected).map_err(|m| m.to_string())
}

pub fn array_length(w: &World, expected: &str) -> Result<(), String> {
    let want: usize = expected
        .parse()
        .map_err(|_| format!("invalid length {expected:?}"))?;
    let v = last(w)?.json()?;
    let arr = v.as_array().ok_or("response body is not an array")?;
    if arr.len() == want {
        Ok(())
    } else {
        Err(format!(
            "    expected: array of length {want}\n    actual:   length {}",
            arr.len()
        ))
    }
}

pub fn json_node_exists(w: &World, p: &str) -> Result<(), String> {
    let v = last(w)?.json()?;
    path::read(&v, p).map(|_| ())
}
