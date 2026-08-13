use crate::json::path;
use crate::world::World;

pub fn set_variable(w: &mut World, name: &str, value: &str, global: bool) -> Result<(), String> {
    if global {
        w.vars.set_global(name, value.to_string());
    } else {
        w.vars.set(name, value.to_string());
    }
    Ok(())
}

/// Scalar values are stored as-is; strings without JSON quotes.
fn scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn extract_from_json(w: &mut World, p: &str, name: &str, global: bool) -> Result<(), String> {
    let ex = w.http.last().ok_or("no request has been sent yet")?;
    let v = ex.json()?;
    let found = path::read(&v, p)?;
    let value = scalar(found);
    if global {
        w.vars.set_global(name, value);
    } else {
        w.vars.set(name, value);
    }
    Ok(())
}

pub fn extract_from_cookies(w: &mut World, cookie: &str, name: &str, global: bool) -> Result<(), String> {
    let ex = w.http.last().ok_or("no request has been sent yet")?;
    let value = ex
        .set_cookie(cookie)
        .ok_or_else(|| format!("cookie {cookie:?} not found in the response"))?;
    if global {
        w.vars.set_global(name, value);
    } else {
        w.vars.set(name, value);
    }
    Ok(())
}

pub fn variable_equals(w: &World, name: &str, expected: &str, negate: bool) -> Result<(), String> {
    let got = w.vars.get(name).ok_or_else(|| format!("variable {name:?} is not set"))?;
    let equal = got == expected;
    match (equal, negate) {
        (true, false) | (false, true) => Ok(()),
        (false, false) => Err(format!("    expected: {expected}\n    actual:   {got}")),
        (true, true) => Err(format!("value must not equal {expected:?}, but it does")),
    }
}
