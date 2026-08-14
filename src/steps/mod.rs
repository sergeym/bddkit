pub mod api;
pub mod assert;
pub mod vars;

use crate::world::World;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepId {
    // request
    SetRequestHeader,
    AddRequestHeader,
    SetQueryParam,
    SetRequestBody,
    EmptyRequestBody,
    SetFormParams,
    RequestPath,
    RequestPathWithMethod,
    // response checks
    ResponseCode,
    ResponseBodyContainsJson,
    ResponseBodyEqualsJson,
    ResponseArrayLength,
    ResponseHeader,
    JsonNodeExists,
    // variables
    SetVariable,
    SetVariableGlobal,
    ExtractFromJson,
    ExtractFromJsonGlobal,
    ExtractFromCookies,
    ExtractFromCookiesGlobal,
    VariableEquals,
    VariableNotEquals,
}

pub struct StepDef {
    pub id: StepId,
    pub pattern: &'static str,
}

pub const BUILTIN_STEPS: &[StepDef] = &[
    StepDef {
        id: StepId::SetRequestHeader,
        pattern: r#"^the "([^"]*)" request header is "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::AddRequestHeader,
        pattern: r#"^I add "([^"]*)" to the "([^"]*)" request header$"#,
    },
    StepDef {
        id: StepId::SetQueryParam,
        pattern: r#"^the query parameter "([^"]*)" is "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::SetRequestBody,
        pattern: r#"^the request body is:$"#,
    },
    StepDef {
        id: StepId::EmptyRequestBody,
        pattern: r#"^the request body is empty$"#,
    },
    StepDef {
        id: StepId::SetFormParams,
        pattern: r#"^the request form parameters are:$"#,
    },
    StepDef {
        id: StepId::RequestPathWithMethod,
        pattern: r#"^I request "([^"]*)" using HTTP (GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)$"#,
    },
    StepDef {
        id: StepId::RequestPath,
        pattern: r#"^I request "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::ResponseCode,
        pattern: r#"^the response code is (\d+)$"#,
    },
    StepDef {
        id: StepId::ResponseBodyContainsJson,
        pattern: r#"^the response body contains JSON:$"#,
    },
    StepDef {
        id: StepId::ResponseBodyEqualsJson,
        pattern: r#"^the response body equals JSON:$"#,
    },
    StepDef {
        id: StepId::ResponseArrayLength,
        pattern: r#"^the response body is a JSON array of length (\d+)$"#,
    },
    StepDef {
        id: StepId::ResponseHeader,
        pattern: r#"^the "([^"]*)" response header is "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::JsonNodeExists,
        pattern: r#"^the JSON node "([^"]*)" should exist$"#,
    },
    StepDef {
        id: StepId::SetVariableGlobal,
        pattern: r#"^set variable "([^"]*)" to "([^"]*)" global$"#,
    },
    StepDef {
        id: StepId::SetVariable,
        pattern: r#"^set variable "([^"]*)" to "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::ExtractFromJsonGlobal,
        pattern: r#"^extract "([^"]*)" from JSON as "([^"]*)" global$"#,
    },
    StepDef {
        id: StepId::ExtractFromJson,
        pattern: r#"^extract "([^"]*)" from JSON as "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::ExtractFromCookiesGlobal,
        pattern: r#"^extract "([^"]*)" from cookies as "([^"]*)" global$"#,
    },
    StepDef {
        id: StepId::ExtractFromCookies,
        pattern: r#"^extract "([^"]*)" from cookies as "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::VariableNotEquals,
        pattern: r#"^variable "([^"]*)" should not be equal to "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::VariableEquals,
        pattern: r#"^variable "([^"]*)" should be equal to "([^"]*)"$"#,
    },
];

pub struct Registry {
    entries: Vec<(StepId, Regex)>,
}

impl Registry {
    pub fn new() -> Result<Self, String> {
        let mut entries = Vec::with_capacity(BUILTIN_STEPS.len());
        for def in BUILTIN_STEPS {
            let re = Regex::new(def.pattern)
                .map_err(|e| format!("invalid step pattern {:?}: {e}", def.pattern))?;
            entries.push((def.id, re));
        }
        Ok(Self { entries })
    }

    /// Returns `Ok(None)` if the step is unknown, and `Err` if it is ambiguous.
    /// Ambiguity is an error, not "first wins": a silently shadowed step
    /// is more expensive to debug than a failed start.
    pub fn find(&self, text: &str) -> Result<Option<(StepId, Vec<String>)>, String> {
        let mut hits: Vec<(StepId, Vec<String>)> = Vec::new();
        for (id, re) in &self.entries {
            if let Some(c) = re.captures(text) {
                let caps = c
                    .iter()
                    .skip(1)
                    .map(|g| g.map(|m| m.as_str().to_string()).unwrap_or_default())
                    .collect();
                hits.push((*id, caps));
            }
        }
        match hits.len() {
            0 => Ok(None),
            1 => Ok(Some(hits.remove(0))),
            _ => Err(format!(
                "step {text:?} matches several definitions: {:?}",
                hits.iter().map(|(id, _)| id).collect::<Vec<_>>()
            )),
        }
    }
}

/// Step arguments after interpolation.
pub struct Args {
    pub caps: Vec<String>,
    pub docstring: Option<String>,
    pub table: Option<Vec<Vec<String>>>,
}

impl Args {
    fn cap(&self, i: usize) -> &str {
        self.caps.get(i).map(String::as_str).unwrap_or("")
    }
}

/// One `match` instead of boxed futures and trait objects.
pub async fn dispatch(w: &mut World, id: StepId, a: &Args) -> Result<(), String> {
    match id {
        StepId::SetRequestHeader => api::set_header(w, a.cap(0), a.cap(1)),
        // The step names the value before the name: `I add "V" to the "H" request header`.
        StepId::AddRequestHeader => api::add_header(w, a.cap(1), a.cap(0)),
        StepId::SetQueryParam => api::set_query(w, a.cap(0), a.cap(1)),
        StepId::SetRequestBody => api::set_body(w, a.docstring.as_ref()),
        StepId::EmptyRequestBody => api::clear_body(w),
        StepId::SetFormParams => api::set_form(w, a.table.as_ref()),
        StepId::RequestPath => api::request(w, a.cap(0), "GET").await,
        StepId::RequestPathWithMethod => {
            let (p, m) = (a.cap(0).to_string(), a.cap(1).to_string());
            api::request(w, &p, &m).await
        }
        StepId::ResponseCode => assert::response_code(w, a.cap(0)),
        StepId::ResponseHeader => assert::response_header(w, a.cap(0), a.cap(1)),
        StepId::ResponseBodyContainsJson => assert::body_contains_json(w, a.docstring.as_ref()),
        StepId::ResponseBodyEqualsJson => assert::body_equals_json(w, a.docstring.as_ref()),
        StepId::ResponseArrayLength => assert::array_length(w, a.cap(0)),
        StepId::JsonNodeExists => assert::json_node_exists(w, a.cap(0)),
        StepId::SetVariable => vars::set_variable(w, a.cap(0), a.cap(1), false),
        StepId::SetVariableGlobal => vars::set_variable(w, a.cap(0), a.cap(1), true),
        StepId::ExtractFromJson => vars::extract_from_json(w, a.cap(0), a.cap(1), false),
        StepId::ExtractFromJsonGlobal => vars::extract_from_json(w, a.cap(0), a.cap(1), true),
        StepId::ExtractFromCookies => vars::extract_from_cookies(w, a.cap(0), a.cap(1), false),
        StepId::ExtractFromCookiesGlobal => vars::extract_from_cookies(w, a.cap(0), a.cap(1), true),
        StepId::VariableEquals => vars::variable_equals(w, a.cap(0), a.cap(1), false),
        StepId::VariableNotEquals => vars::variable_equals(w, a.cap(0), a.cap(1), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        Registry::new().unwrap()
    }

    #[test]
    fn all_builtin_patterns_compile() {
        assert!(Registry::new().is_ok());
    }

    #[test]
    fn no_two_builtin_steps_are_ambiguous() {
        // Every pattern is checked against all the others on concrete examples.
        let samples = [
            r#"the "Accept" request header is "application/json""#,
            r#"I add "b" to the "Accept" request header"#,
            r#"the query parameter "q" is "1""#,
            "the request body is:",
            "the request body is empty",
            "the request form parameters are:",
            r#"I request "/ping" using HTTP POST"#,
            r#"I request "/ping""#,
            "the response code is 200",
            "the response body contains JSON:",
            "the response body equals JSON:",
            "the response body is a JSON array of length 3",
            r#"the "X-Trace" response header is "abc""#,
            r#"the JSON node "data.id" should exist"#,
            r#"set variable "a" to "1""#,
            r#"set variable "a" to "1" global"#,
            r#"extract "id" from JSON as "userId""#,
            r#"extract "id" from JSON as "userId" global"#,
            r#"extract "jwt" from cookies as "t""#,
            r#"extract "jwt" from cookies as "t" global"#,
            r#"variable "a" should be equal to "1""#,
            r#"variable "a" should not be equal to "1""#,
        ];
        let r = reg();
        for s in samples {
            match r.find(s) {
                Ok(Some(_)) => {}
                Ok(None) => panic!("step not recognized: {s}"),
                Err(e) => panic!("ambiguity: {e}"),
            }
        }
    }

    #[test]
    fn captures_path_and_method() {
        let (id, caps) = reg()
            .find(r#"I request "/api/v1/x" using HTTP PATCH"#)
            .unwrap()
            .unwrap();
        assert_eq!(id, StepId::RequestPathWithMethod);
        assert_eq!(caps, vec!["/api/v1/x".to_string(), "PATCH".to_string()]);
    }

    #[test]
    fn bare_request_defaults_to_get_variant() {
        let (id, caps) = reg().find(r#"I request "/ping""#).unwrap().unwrap();
        assert_eq!(id, StepId::RequestPath);
        assert_eq!(caps, vec!["/ping".to_string()]);
    }

    #[test]
    fn global_suffix_picks_the_global_variant() {
        let (id, _) = reg()
            .find(r#"set variable "a" to "1" global"#)
            .unwrap()
            .unwrap();
        assert_eq!(id, StepId::SetVariableGlobal);
        let (id, _) = reg().find(r#"set variable "a" to "1""#).unwrap().unwrap();
        assert_eq!(id, StepId::SetVariable);
    }

    #[test]
    fn unknown_method_is_not_a_known_step() {
        assert!(
            reg()
                .find(r#"I request "/ping" using HTTP POSTT"#)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unknown_step_returns_none() {
        assert!(reg().find("I refund the order").unwrap().is_none());
    }
}
