pub mod api;
pub mod assert;
pub mod db;
pub mod debug;
pub mod vars;

use crate::macros::{MacroCatalog, MacroDef};
use crate::world::World;
use regex::Regex;
use std::collections::{HashSet, VecDeque};
use std::sync::LazyLock;

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
    UseApi,
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
    // database
    UseConnection,
    DebugOn,
    DebugOff,
    HaveWith,
    HaveWhere,
    HaveMulti,
    Update,
    DeleteWhere,
    DeleteAll,
    ExtractFromDb,
    ShouldHaveWith,
    ShouldHaveTable,
    ShouldNotHaveWith,
    ShouldNotHaveTable,
    CallProcedure,
    CallFunction,
    GetSequence,
    Sleep,
    ShowAllVariables,
    ShowVariable,
    PrintResponseHeaders,
    PrintResponseBody,
    PrintResponseBodyAsPath,
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
    StepDef {
        id: StepId::UseApi,
        pattern: r#"^I use "([^"]*)" api$"#,
    },
    StepDef {
        id: StepId::UseConnection,
        pattern: r#"^I use "([^"]*)" connection$"#,
    },
    StepDef {
        id: StepId::DebugOn,
        pattern: r#"^I am in debug mode$"#,
    },
    StepDef {
        id: StepId::DebugOff,
        pattern: r#"^I am not in debug mode$"#,
    },
    StepDef {
        id: StepId::HaveWhere,
        pattern: r#"^I have "([^"]*)" where:$"#,
    },
    StepDef {
        id: StepId::HaveWith,
        pattern: r#"^I have "([^"]*)" with "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::HaveMulti,
        pattern: r#"^I have:$"#,
    },
    StepDef {
        id: StepId::Update,
        pattern: r#"^I update "([^"]*)" with "([^"]*)" where "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::DeleteAll,
        pattern: r#"^I delete all "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::DeleteWhere,
        pattern: r#"^I delete "([^"]*)" where "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::ExtractFromDb,
        pattern: r#"^I extract "([^"]*)" from "([^"]*)" with "([^"]*)" as "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::ShouldNotHaveTable,
        pattern: r#"^I should not have "([^"]*)" with:$"#,
    },
    StepDef {
        id: StepId::ShouldNotHaveWith,
        pattern: r#"^I should not have "([^"]*)" with "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::ShouldHaveTable,
        pattern: r#"^I should have "([^"]*)" with:$"#,
    },
    StepDef {
        id: StepId::ShouldHaveWith,
        pattern: r#"^I should have "([^"]*)" with "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::CallProcedure,
        pattern: r#"^I call procedure "([^"]*)" with "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::CallFunction,
        pattern: r#"^I call function "([^"]*)" with "([^"]*)" as "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::GetSequence,
        pattern: r#"^I get next value of sequence "([^"]*)" as "([^"]*)"$"#,
    },
    StepDef {
        id: StepId::Sleep,
        pattern: r#"^I sleep "(\d+)" seconds$"#,
    },
    StepDef {
        id: StepId::ShowAllVariables,
        pattern: r#"^Show all variables$"#,
    },
    StepDef {
        id: StepId::ShowVariable,
        pattern: r#"^Show "([^"]*)" variable$"#,
    },
    StepDef {
        id: StepId::PrintResponseHeaders,
        pattern: r#"^Print response headers$"#,
    },
    StepDef {
        id: StepId::PrintResponseBody,
        pattern: r#"^Print response body$"#,
    },
    StepDef {
        id: StepId::PrintResponseBodyAsPath,
        pattern: r#"^Print response body as "([^"]*)"$"#,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepTarget {
    Builtin(StepId),
    Macro(usize),
}

impl PartialEq<StepId> for StepTarget {
    fn eq(&self, other: &StepId) -> bool {
        matches!(self, Self::Builtin(id) if id == other)
    }
}

#[derive(Debug)]
pub struct Registry {
    entries: Vec<(StepTarget, Regex)>,
    macros: Vec<MacroDef>,
}

impl Registry {
    #[cfg(test)]
    pub fn new() -> Result<Self, String> {
        Self::with_macros(MacroCatalog {
            definitions: Vec::new(),
        })
    }

    pub fn with_macros(catalog: MacroCatalog) -> Result<Self, String> {
        let mut entries = Vec::with_capacity(BUILTIN_STEPS.len());
        for def in BUILTIN_STEPS {
            let re = Regex::new(def.pattern)
                .map_err(|e| format!("invalid step pattern {:?}: {e}", def.pattern))?;
            entries.push((StepTarget::Builtin(def.id), re));
        }
        for (index, definition) in catalog.definitions.iter().enumerate() {
            entries.push((StepTarget::Macro(index), definition.regex.clone()));
        }
        let registry = Self {
            entries,
            macros: catalog.definitions,
        };
        registry.validate_macros()?;
        Ok(registry)
    }

    /// Returns `Ok(None)` if the step is unknown, and `Err` if it is ambiguous.
    /// Ambiguity is an error, not "first wins": a silently shadowed step
    /// is more expensive to debug than a failed start.
    pub fn find(&self, text: &str) -> Result<Option<(StepTarget, Vec<String>)>, String> {
        let mut hits: Vec<(StepTarget, Vec<String>)> = Vec::new();
        for (target, re) in &self.entries {
            if let Some(c) = re.captures(text) {
                let caps = c
                    .iter()
                    .skip(1)
                    .map(|g| g.map(|m| m.as_str().to_string()).unwrap_or_default())
                    .collect();
                hits.push((*target, caps));
            }
        }
        match hits.len() {
            0 => Ok(None),
            1 => Ok(Some(hits.remove(0))),
            _ => Err(format!(
                "step {text:?} matches several definitions: {:?}",
                hits.iter().map(|(target, _)| target).collect::<Vec<_>>()
            )),
        }
    }

    pub fn macro_def(&self, index: usize) -> &MacroDef {
        &self.macros[index]
    }

    fn validate_macros(&self) -> Result<(), String> {
        for (left_index, left) in self.macros.iter().enumerate() {
            for builtin in BUILTIN_STEPS {
                if builtin_patterns(builtin.pattern)
                    .iter()
                    .any(|pattern| patterns_overlap(&macro_pattern(&left.step), pattern))
                {
                    return Err(format!(
                        "macro step {:?} from {}:{} conflicts with builtin step {:?}",
                        left.step,
                        left.source.display(),
                        left.line,
                        builtin.pattern
                    ));
                }
            }
            for right in self.macros.iter().skip(left_index + 1) {
                if patterns_overlap(&macro_pattern(&left.step), &macro_pattern(&right.step)) {
                    return Err(format!(
                        "macro step {:?} from {}:{} conflicts with {:?} from {}:{}",
                        left.step,
                        left.source.display(),
                        left.line,
                        right.step,
                        right.source.display(),
                        right.line,
                    ));
                }
            }
        }

        let mut graph = vec![Vec::new(); self.macros.len()];
        for (index, definition) in self.macros.iter().enumerate() {
            for step in &definition.body {
                match self.find(&step.text)? {
                    Some((StepTarget::Builtin(_), _)) => {}
                    Some((StepTarget::Macro(_), _)) if step.docstring.is_some() => {
                        return Err(format!(
                            "calling macro {:?} with a docstring inside macro {:?} from {}:{} is not supported",
                            step.text,
                            definition.step,
                            definition.source.display(),
                            definition.line,
                        ));
                    }
                    Some((StepTarget::Macro(target), _)) => graph[index].push(target),
                    None => {
                        return Err(format!(
                            "unknown step {:?} in macro {:?} from {}:{}",
                            step.text,
                            definition.step,
                            definition.source.display(),
                            definition.line,
                        ));
                    }
                }
            }
        }

        let mut visited = vec![false; self.macros.len()];
        let mut visiting = vec![false; self.macros.len()];
        let mut path = Vec::new();
        for index in 0..self.macros.len() {
            self.visit(index, &graph, &mut visited, &mut visiting, &mut path)?;
        }
        let mut depths = vec![None; self.macros.len()];
        for index in 0..self.macros.len() {
            if macro_depth(index, &graph, &mut depths) > 16 {
                return Err(format!(
                    "macro nesting exceeds 16 starting from step {:?}",
                    self.macros[index].step
                ));
            }
        }
        Ok(())
    }

    fn visit(
        &self,
        index: usize,
        graph: &[Vec<usize>],
        visited: &mut [bool],
        visiting: &mut [bool],
        path: &mut Vec<usize>,
    ) -> Result<(), String> {
        if visiting[index] {
            let start = path.iter().position(|item| *item == index).unwrap_or(0);
            let mut cycle: Vec<&str> = path[start..]
                .iter()
                .map(|item| self.macros[*item].step.as_str())
                .collect();
            cycle.push(self.macros[index].step.as_str());
            return Err(format!("cycle in macros: {}", cycle.join(" → ")));
        }
        if visited[index] {
            return Ok(());
        }
        visiting[index] = true;
        path.push(index);
        for child in &graph[index] {
            self.visit(*child, graph, visited, visiting, path)?;
        }
        path.pop();
        visiting[index] = false;
        visited[index] = true;
        Ok(())
    }
}

fn macro_depth(index: usize, graph: &[Vec<usize>], memo: &mut [Option<usize>]) -> usize {
    if let Some(depth) = memo[index] {
        return depth;
    }
    let depth = 1 + graph[index]
        .iter()
        .map(|child| macro_depth(*child, graph, memo))
        .max()
        .unwrap_or(0);
    memo[index] = Some(depth);
    depth
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Any,
    NonQuote,
    Digit,
    Exact(char),
}

static DIGIT_CHAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d$").expect("constant digit regex"));

#[derive(Clone, Copy, Debug)]
enum PatternToken {
    One(CharClass),
    Star(CharClass),
}

static MACRO_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{[A-Za-z_][A-Za-z0-9_]*\}").expect("constant macro parameter regex")
});

fn macro_pattern(template: &str) -> Vec<PatternToken> {
    let mut tokens = Vec::new();
    let mut last = 0;
    for parameter in MACRO_PARAM.find_iter(template) {
        tokens.extend(
            template[last..parameter.start()]
                .chars()
                .map(|char_| PatternToken::One(CharClass::Exact(char_))),
        );
        tokens.push(PatternToken::Star(CharClass::Any));
        last = parameter.end();
    }
    tokens.extend(
        template[last..]
            .chars()
            .map(|char_| PatternToken::One(CharClass::Exact(char_))),
    );
    tokens
}

fn builtin_patterns(pattern: &str) -> Vec<Vec<PatternToken>> {
    const METHODS: &str = "(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)";
    let variants: Vec<String> = if pattern.contains(METHODS) {
        ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
            .iter()
            .map(|method| pattern.replace(METHODS, method))
            .collect()
    } else {
        vec![pattern.to_string()]
    };
    variants
        .iter()
        .map(|variant| {
            let source = variant.trim_start_matches('^').trim_end_matches('$');
            let mut tokens = Vec::new();
            let mut rest = source;
            while !rest.is_empty() {
                if let Some(tail) = rest.strip_prefix(r#"([^"]*)"#) {
                    tokens.push(PatternToken::Star(CharClass::NonQuote));
                    rest = tail;
                } else if let Some(tail) = rest.strip_prefix(r"(\d+)") {
                    tokens.push(PatternToken::One(CharClass::Digit));
                    tokens.push(PatternToken::Star(CharClass::Digit));
                    rest = tail;
                } else {
                    let char_ = rest.chars().next().expect("string is not empty");
                    tokens.push(PatternToken::One(CharClass::Exact(char_)));
                    rest = &rest[char_.len_utf8()..];
                }
            }
            tokens
        })
        .collect()
}

fn patterns_overlap(left: &[PatternToken], right: &[PatternToken]) -> bool {
    let mut queue = VecDeque::from([(0usize, 0usize)]);
    let mut visited = HashSet::new();
    while let Some((left_pos, right_pos)) = queue.pop_front() {
        if !visited.insert((left_pos, right_pos)) {
            continue;
        }
        if left_pos == left.len() && right_pos == right.len() {
            return true;
        }
        if matches!(left.get(left_pos), Some(PatternToken::Star(_))) {
            queue.push_back((left_pos + 1, right_pos));
        }
        if matches!(right.get(right_pos), Some(PatternToken::Star(_))) {
            queue.push_back((left_pos, right_pos + 1));
        }
        let Some(left_token) = left.get(left_pos) else {
            continue;
        };
        let Some(right_token) = right.get(right_pos) else {
            continue;
        };
        let (left_class, left_next) = consumed(*left_token, left_pos);
        let (right_class, right_next) = consumed(*right_token, right_pos);
        if classes_overlap(left_class, right_class) {
            queue.push_back((left_next, right_next));
        }
    }
    false
}

fn consumed(token: PatternToken, position: usize) -> (CharClass, usize) {
    match token {
        PatternToken::One(class) => (class, position + 1),
        PatternToken::Star(class) => (class, position),
    }
}

fn classes_overlap(left: CharClass, right: CharClass) -> bool {
    use CharClass::{Any, Digit, Exact, NonQuote};
    match (left, right) {
        (Any, _)
        | (_, Any)
        | (NonQuote, NonQuote)
        | (NonQuote, Digit)
        | (Digit, NonQuote)
        | (Digit, Digit) => true,
        (NonQuote, Exact(char_)) | (Exact(char_), NonQuote) => char_ != '"',
        (Digit, Exact(char_)) | (Exact(char_), Digit) => DIGIT_CHAR.is_match(&char_.to_string()),
        (Exact(left), Exact(right)) => left == right,
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
        StepId::UseApi => api::use_api(w, a.cap(0)),
        StepId::UseConnection => db::use_connection(w, a.cap(0)),
        StepId::DebugOn => db::debug_on(w),
        StepId::DebugOff => db::debug_off(w),
        StepId::HaveWith => db::have_with(w, a.cap(0), a.cap(1)).await,
        StepId::HaveWhere => db::have_where(w, a.cap(0), a.table.as_ref()).await,
        StepId::HaveMulti => db::have_multi(w, a.table.as_ref()).await,
        StepId::Update => db::update(w, a.cap(0), a.cap(1), a.cap(2)).await,
        StepId::DeleteWhere => db::delete_where(w, a.cap(0), a.cap(1)).await,
        StepId::DeleteAll => db::delete_all(w, a.cap(0)).await,
        StepId::ExtractFromDb => db::extract_from_db(w, a.cap(0), a.cap(1), a.cap(2), a.cap(3)).await,
        StepId::ShouldHaveWith => db::should_have(w, a.cap(0), a.cap(1)).await,
        StepId::ShouldHaveTable => db::should_have_table(w, a.cap(0), a.table.as_ref()).await,
        StepId::ShouldNotHaveWith => db::should_not_have(w, a.cap(0), a.cap(1)).await,
        StepId::ShouldNotHaveTable => db::should_not_have_table(w, a.cap(0), a.table.as_ref()).await,
        StepId::CallProcedure => db::call_procedure(w, a.cap(0), a.cap(1)).await,
        StepId::CallFunction => db::call_function(w, a.cap(0), a.cap(1), a.cap(2)).await,
        StepId::GetSequence => db::get_sequence(w, a.cap(0), a.cap(1)).await,
        StepId::Sleep => sleep(a.cap(0)).await,
        StepId::ShowAllVariables => show_all_variables(w),
        StepId::ShowVariable => show_variable(w, a.cap(0)),
        StepId::PrintResponseHeaders => debug::print_headers(w),
        StepId::PrintResponseBody => debug::print_body(w),
        StepId::PrintResponseBodyAsPath => debug::print_body_as(w, a.cap(0)),
    }
}

async fn sleep(secs: &str) -> Result<(), String> {
    let n = secs.parse::<u64>()
        .map_err(|_| format!("not a number: {secs}"))?;
    tokio::time::sleep(std::time::Duration::from_secs(n)).await;
    Ok(())
}

fn show_all_variables(w: &World) -> Result<(), String> {
    eprintln!("=== All variables ===");
    for (name, value) in w.vars.all_vars() {
        eprintln!("{}: {}", name, value);
    }
    Ok(())
}

fn show_variable(w: &World, name: &str) -> Result<(), String> {
    match w.vars.get(name) {
        Some(v) => {
            eprintln!("{}: {}", name, v);
            Ok(())
        }
        None => Err(format!("variable not found: {name}")),
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
            r#"I use "main" connection"#,
            r#"I use "billing" api"#,
            "I am in debug mode",
            "I am not in debug mode",
            r#"I have "users" where:"#,
            r#"I have "users" with "email: a@b.net""#,
            "I have:",
            r#"I update "companies" with "name: x" where "id: 1""#,
            r#"I delete "companies" where "slug: x""#,
            r#"I delete all "companies""#,
            r#"I extract "id" from "companies" with "slug: x" as "cid""#,
            r#"I call procedure "p" with "a: 1""#,
            r#"I call function "f" with "a: 1" as "r""#,
            r#"I get next value of sequence "s" as "n""#,
            r#"I sleep "5" seconds"#,
            "Show all variables",
            r#"Show "userId" variable"#,
            "Print response headers",
            "Print response body",
            r#"Print response body as "status""#,
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
    fn the_use_api_step_resolves_to_its_builtin() {
        let (target, caps) = reg()
            .find(r#"I use "billing" api"#)
            .expect("pattern is not ambiguous")
            .expect("step is declared");
        assert!(target == StepId::UseApi, "step must resolve to UseApi");
        assert_eq!(caps, vec!["billing".to_string()]);
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
