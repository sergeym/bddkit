use crate::feature::{LoadedFeature, expand_outlines};
use crate::steps::Registry;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Problem {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "  {}:{}\n    {}", self.file.display(), self.line, self.message)
    }
}

/// Matches every step of every file before the first request. Returns ALL
/// problems at once — a run that fails mid-way over a typo costs more
/// than a full check that takes milliseconds.
pub fn check(features: &[LoadedFeature], reg: &Registry) -> Vec<Problem> {
    let mut problems = Vec::new();
    for lf in features {
        let mut all_steps: Vec<(String, usize)> = Vec::new();
        if let Some(bg) = &lf.feature.background {
            for s in &bg.steps {
                all_steps.push((s.value.clone(), s.position.line));
            }
        }
        for sc in &lf.feature.scenarios {
            for ex in expand_outlines(sc) {
                for st in ex.steps {
                    all_steps.push((st.text, st.line));
                }
            }
        }
        for (text, line) in all_steps {
            match reg.find(&text) {
                Ok(Some(_)) => {}
                Ok(None) => problems.push(Problem {
                    file: lf.path.clone(),
                    line,
                    message: format!("unknown step: {text:?}"),
                }),
                Err(e) => problems.push(Problem { file: lf.path.clone(), line, message: e }),
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::{LoadedFeature, parse_str};

    fn loaded(src: &str) -> LoadedFeature {
        LoadedFeature { path: PathBuf::from("t.feature"), feature: parse_str(src).unwrap() }
    }

    #[test]
    fn accepts_a_file_of_known_steps() {
        let lf = loaded("\
Feature: f
  Background:
    Given the \"Accept\" request header is \"application/json\"
  Scenario: s
    When I request \"/ping\" using HTTP POST
    Then the response code is 200
");
        let p = check(&[lf], &Registry::new().unwrap());
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn reports_unknown_step_with_file_and_line() {
        let lf = loaded("Feature: f\n  Scenario: s\n    When I refund the order\n");
        let p = check(&[lf], &Registry::new().unwrap());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].line, 3);
        assert!(p[0].message.contains("I refund the order"), "{}", p[0].message);
    }

    #[test]
    fn reports_every_problem_not_just_the_first() {
        let lf = loaded("\
Feature: f
  Scenario: s
    When I refund the order
    Then I ship the order
");
        let p = check(&[lf], &Registry::new().unwrap());
        assert_eq!(p.len(), 2, "all problems must be reported at once");
    }

    #[test]
    fn checks_background_steps_too() {
        let lf = loaded("Feature: f\n  Background:\n    Given I refund the order\n  Scenario: s\n    Then the response code is 200\n");
        let p = check(&[lf], &Registry::new().unwrap());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].line, 3);
    }

    #[test]
    fn checks_expanded_outline_steps() {
        let lf = loaded("\
Feature: f
  Scenario Outline: s
    When I request \"/ping\" using HTTP <method>
    Examples:
      | method |
      | POSTT  |
");
        let p = check(&[lf], &Registry::new().unwrap());
        assert_eq!(p.len(), 1, "a typo in the method must surface before the run");
    }

    #[test]
    fn steps_with_variables_validate_without_values() {
        // Substitution happens in arguments, so matching does not require values.
        let lf = loaded("Feature: f\n  Scenario: s\n    When I request \"/users/<<userId>>\" using HTTP GET\n");
        let p = check(&[lf], &Registry::new().unwrap());
        assert!(p.is_empty(), "{p:?}");
    }
}
