use crate::http::ReplayError;
use crate::json::{matcher, path};
use crate::polling::{AttemptError, AttemptResult};
use crate::world::World;

pub(super) async fn replay_response(w: &mut World, attempt: u64) -> AttemptResult {
    if attempt == 0 {
        return Ok(());
    }
    w.http.replay_last().await.map_err(|error| match error {
        ReplayError::NotYet(message) => AttemptError::NotYet(message),
        ReplayError::Fatal(message) => AttemptError::Fatal(message),
    })
}

pub(crate) fn last(w: &World) -> Result<&crate::http::Exchange, String> {
    w.http
        .last()
        .ok_or_else(|| "no request has been sent yet".to_string())
}

pub fn response_code(w: &World, expected: &str) -> AttemptResult {
    let want: u16 = expected
        .parse()
        .map_err(|_| AttemptError::Fatal(format!("invalid code {expected:?}")))?;
    let got = last(w).map_err(AttemptError::Fatal)?.status;
    if got == want {
        Ok(())
    } else {
        Err(AttemptError::NotYet(format!(
            "    expected: {want}\n    actual:   {got}"
        )))
    }
}

pub fn response_header(w: &World, name: &str, value: &str) -> AttemptResult {
    let ex = last(w).map_err(AttemptError::Fatal)?;
    let got = ex
        .resp_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str());
    match got {
        Some(v) if v == value => Ok(()),
        Some(v) => Err(AttemptError::NotYet(format!(
            "    expected: {value}\n    actual:   {v}"
        ))),
        None => Err(AttemptError::NotYet(format!(
            "response header {name:?} is missing"
        ))),
    }
}

pub fn body_contains_json(w: &World, docstring: Option<&String>) -> AttemptResult {
    let expected_src =
        docstring.ok_or_else(|| AttemptError::Fatal("step requires a doc string with JSON".to_string()))?;
    let expected: serde_json::Value = serde_json::from_str(expected_src)
        .map_err(|e| AttemptError::Fatal(format!("expected JSON is invalid: {e}")))?;
    let actual = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    matcher::contains(&actual, &expected).map_err(|m| AttemptError::NotYet(m.to_string()))
}

pub fn body_equals_json(w: &World, docstring: Option<&String>) -> AttemptResult {
    let expected_src =
        docstring.ok_or_else(|| AttemptError::Fatal("step requires a doc string with JSON".to_string()))?;
    let expected: serde_json::Value = serde_json::from_str(expected_src)
        .map_err(|e| AttemptError::Fatal(format!("expected JSON is invalid: {e}")))?;
    let actual = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    matcher::equals(&actual, &expected).map_err(|m| AttemptError::NotYet(m.to_string()))
}

pub fn array_length(w: &World, expected: &str) -> AttemptResult {
    let want: usize = expected
        .parse()
        .map_err(|_| AttemptError::Fatal(format!("invalid length {expected:?}")))?;
    let v = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    let arr = v
        .as_array()
        .ok_or_else(|| AttemptError::NotYet("response body is not an array".to_string()))?;
    if arr.len() == want {
        Ok(())
    } else {
        Err(AttemptError::NotYet(format!(
            "    expected: array of length {want}\n    actual:   length {}",
            arr.len()
        )))
    }
}

pub fn json_node_exists(w: &World, p: &str) -> AttemptResult {
    path::validate(p).map_err(AttemptError::Fatal)?;
    let v = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    path::read(&v, p).map(|_| ()).map_err(AttemptError::NotYet)
}
