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
    check_array_length(&v, want)
}

/// Issue #22, item 5: "body is not an array at all" and "array has the wrong length"
/// are distinguishable failures already — kept as two separate messages here rather
/// than a new step.
fn check_array_length(v: &serde_json::Value, want: usize) -> AttemptResult {
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

pub fn json_node_not_exists(w: &World, p: &str) -> AttemptResult {
    path::validate(p).map_err(AttemptError::Fatal)?;
    let v = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    check_not_exists(&v, p)
}

/// A `null` node exists — this must not treat `null` as absent (issue #22, item 1).
fn check_not_exists(v: &serde_json::Value, p: &str) -> AttemptResult {
    match path::read(v, p) {
        Err(_) => Ok(()),
        Ok(node) => Err(AttemptError::NotYet(format!(
            "    expected: node {p:?} to not exist\n    actual:   {node}"
        ))),
    }
}

pub fn json_node_is_null(w: &World, p: &str) -> AttemptResult {
    path::validate(p).map_err(AttemptError::Fatal)?;
    let v = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    check_is_null(&v, p)
}

pub fn json_node_not_null(w: &World, p: &str) -> AttemptResult {
    path::validate(p).map_err(AttemptError::Fatal)?;
    let v = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    check_not_null(&v, p)
}

/// "does not exist" and "exists but not null" are different bugs (issue #22, item 2) —
/// a missing node is always its own NotYet message, never folded into "not null".
fn check_is_null(v: &serde_json::Value, p: &str) -> AttemptResult {
    match path::read(v, p) {
        Err(_) => Err(AttemptError::NotYet(format!("node {p:?} does not exist"))),
        Ok(node) if node.is_null() => Ok(()),
        Ok(node) => Err(AttemptError::NotYet(format!(
            "    expected: null\n    actual:   {node}"
        ))),
    }
}

fn check_not_null(v: &serde_json::Value, p: &str) -> AttemptResult {
    match path::read(v, p) {
        Err(_) => Err(AttemptError::NotYet(format!("node {p:?} does not exist"))),
        Ok(node) if node.is_null() => Err(AttemptError::NotYet(
            "    expected: not null\n    actual:   null".to_string(),
        )),
        Ok(_) => Ok(()),
    }
}

pub fn body_not_contains_json(w: &World, docstring: Option<&String>) -> AttemptResult {
    let expected_src =
        docstring.ok_or_else(|| AttemptError::Fatal("step requires a doc string with JSON".to_string()))?;
    let expected: serde_json::Value = serde_json::from_str(expected_src)
        .map_err(|e| AttemptError::Fatal(format!("expected JSON is invalid: {e}")))?;
    let actual = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    check_not_contains_json(&actual, &expected)
}

fn check_not_contains_json(actual: &serde_json::Value, expected: &serde_json::Value) -> AttemptResult {
    match matcher::contains(actual, expected) {
        Ok(()) => Err(AttemptError::NotYet(format!(
            "    expected: body to NOT contain JSON matching:\n{}\n    actual:   it does",
            serde_json::to_string_pretty(expected).unwrap_or_default()
        ))),
        Err(_) => Ok(()),
    }
}

pub fn json_node_not_contains(w: &World, p: &str, needle: &str) -> AttemptResult {
    path::validate(p).map_err(AttemptError::Fatal)?;
    let v = last(w)
        .map_err(AttemptError::Fatal)?
        .json()
        .map_err(AttemptError::NotYet)?;
    check_not_contains_substring(&v, p, needle)
}

/// A node that isn't a string is a type bug, not a pass — never silently stringified
/// (issue #22, item 4). A missing node has nothing to contain the substring, so it passes.
fn check_not_contains_substring(v: &serde_json::Value, p: &str, needle: &str) -> AttemptResult {
    match path::read(v, p) {
        Err(_) => Ok(()),
        Ok(serde_json::Value::String(s)) if s.contains(needle) => Err(AttemptError::NotYet(format!(
            "    node {p:?} should not contain {needle:?}\n    actual:   {s:?}"
        ))),
        Ok(serde_json::Value::String(_)) => Ok(()),
        Ok(other) => Err(AttemptError::Fatal(format!(
            "node {p:?} is not a string, got: {other}"
        ))),
    }
}

pub fn body_empty(w: &World) -> AttemptResult {
    let ex = last(w).map_err(AttemptError::Fatal)?;
    if ex.body.trim().is_empty() {
        Ok(())
    } else {
        Err(AttemptError::NotYet(format!(
            "    expected: empty body\n    actual:   {:?}",
            ex.body
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_exists_passes_when_the_path_is_missing() {
        assert_eq!(check_not_exists(&json!({"a": 1}), "b"), Ok(()));
    }

    #[test]
    fn not_exists_fails_when_the_path_is_present() {
        assert!(matches!(
            check_not_exists(&json!({"a": 1}), "a"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn not_exists_treats_a_null_value_as_present() {
        // Issue #22, item 1: null is not absence.
        assert!(matches!(
            check_not_exists(&json!({"a": null}), "a"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn is_null_passes_on_a_null_node() {
        assert_eq!(check_is_null(&json!({"a": null}), "a"), Ok(()));
    }

    #[test]
    fn is_null_fails_on_a_non_null_node() {
        assert!(matches!(
            check_is_null(&json!({"a": 1}), "a"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn is_null_fails_when_the_node_is_missing() {
        assert!(matches!(
            check_is_null(&json!({"a": 1}), "b"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn not_null_passes_on_a_non_null_node() {
        assert_eq!(check_not_null(&json!({"a": 1}), "a"), Ok(()));
    }

    #[test]
    fn not_null_fails_on_a_null_node() {
        assert!(matches!(
            check_not_null(&json!({"a": null}), "a"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn not_null_fails_when_the_node_is_missing() {
        // Existence and non-null are different assertions (issue #22, item 2).
        assert!(matches!(
            check_not_null(&json!({"a": 1}), "b"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn not_contains_json_passes_when_the_shape_is_absent() {
        assert_eq!(
            check_not_contains_json(&json!({"a": 1}), &json!({"b": 2})),
            Ok(())
        );
    }

    #[test]
    fn not_contains_json_fails_when_the_shape_matches() {
        assert!(matches!(
            check_not_contains_json(&json!({"a": 1, "b": 2}), &json!({"a": 1})),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn not_contains_substring_passes_when_the_node_is_missing() {
        assert_eq!(
            check_not_contains_substring(&json!({"a": "hello"}), "b", "ell"),
            Ok(())
        );
    }

    #[test]
    fn not_contains_substring_passes_when_absent() {
        assert_eq!(
            check_not_contains_substring(&json!({"a": "hello"}), "a", "xyz"),
            Ok(())
        );
    }

    #[test]
    fn not_contains_substring_fails_when_present() {
        assert!(matches!(
            check_not_contains_substring(&json!({"a": "hello"}), "a", "ell"),
            Err(AttemptError::NotYet(_))
        ));
    }

    #[test]
    fn array_length_on_a_non_array_body_reports_missing_rather_than_a_length_mismatch() {
        // Issue #22, item 5: "not an array" and "wrong length" must stay distinguishable.
        assert_eq!(
            check_array_length(&json!({"a": 1}), 2),
            Err(AttemptError::NotYet(
                "response body is not an array".to_string()
            ))
        );
    }

    #[test]
    fn array_length_on_an_array_of_the_wrong_length_reports_the_mismatch() {
        let err = check_array_length(&json!([1, 2]), 3).unwrap_err();
        assert_ne!(err, AttemptError::NotYet("response body is not an array".to_string()));
    }

    #[test]
    fn not_contains_substring_errors_on_a_non_string_node() {
        assert!(matches!(
            check_not_contains_substring(&json!({"a": 1}), "a", "1"),
            Err(AttemptError::Fatal(_))
        ));
    }
}
