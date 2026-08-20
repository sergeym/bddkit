use crate::feature::{LoadedFeature, TagFilter, expand_outlines};
use crate::steps::{Registry, StepTarget};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Problem {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "  {}:{}\n    {}",
            self.file.display(),
            self.line,
            self.message
        )
    }
}

/// Matches every step of every file before the first request. Returns ALL
/// problems at once — a run that fails midway due to a typo costs more
/// than a full check that takes milliseconds.
pub fn check(features: &[LoadedFeature], reg: &Registry, filter: &TagFilter) -> Vec<Problem> {
    let mut problems = Vec::new();
    for lf in features {
        let mut all_steps: Vec<(String, usize, bool, bool)> = Vec::new();
        if let Some(bg) = &lf.feature.background {
            for s in &bg.steps {
                all_steps.push((
                    s.value.clone(),
                    s.position.line,
                    s.docstring.is_some(),
                    s.table.is_some(),
                ));
            }
        }
        // Background is always checked: it runs before every selected
        // scenario. Filtered-out scenarios are not checked — a typo in something
        // that never runs must not fail the run.
        for sc in &lf.feature.scenarios {
            if !filter.matches(&sc.tags) {
                continue;
            }
            for ex in expand_outlines(sc) {
                for st in ex.steps {
                    all_steps.push((
                        st.text,
                        st.line,
                        st.docstring.is_some(),
                        st.table.is_some(),
                    ));
                }
            }
        }
        for (text, line, has_docstring, has_table) in all_steps {
            match reg.find(&text) {
                Ok(Some((StepTarget::Macro(_), _))) if has_docstring => {
                    problems.push(Problem {
                        file: lf.path.clone(),
                        line,
                        message: "macro call does not support a doc string".into(),
                    });
                }
                Ok(Some((StepTarget::Macro(_), _))) if has_table => problems.push(Problem {
                    file: lf.path.clone(),
                    line,
                    message: "macro call does not support a table".into(),
                }),
                Ok(Some(_)) => {}
                Ok(None) => problems.push(Problem {
                    file: lf.path.clone(),
                    line,
                    message: format!("unknown step: {text:?}"),
                }),
                Err(e) => problems.push(Problem {
                    file: lf.path.clone(),
                    line,
                    message: e,
                }),
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::{LoadedFeature, parse_str};
    use crate::macros::MacroCatalog;

    fn loaded(src: &str) -> LoadedFeature {
        LoadedFeature {
            path: PathBuf::from("t.feature"),
            feature: parse_str(src).unwrap(),
        }
    }

    fn macro_registry(name: &str) -> Registry {
        let path = std::env::temp_dir().join(format!(
            "bddkit-validate-macro-{}-{name}.yaml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "- step: I do business\n  do: [the response code is 200]\n",
        )
        .unwrap();
        Registry::with_macros(MacroCatalog::load(&[path]).unwrap()).unwrap()
    }

    #[test]
    fn accepts_a_file_of_known_steps() {
        let lf = loaded(
            "\
Feature: f
  Background:
    Given the \"Accept\" request header is \"application/json\"
  Scenario: s
    When I request \"/ping\" using HTTP POST
    Then the response code is 200
",
        );
        let p = check(&[lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn reports_unknown_step_with_file_and_line() {
        let lf = loaded("Feature: f\n  Scenario: s\n    When I refund the order\n");
        let p = check(&[lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].line, 3);
        assert!(
            p[0].message.contains("I refund the order"),
            "{}",
            p[0].message
        );
    }

    #[test]
    fn reports_every_problem_not_just_the_first() {
        let lf = loaded(
            "\
Feature: f
  Scenario: s
    When I refund the order
    Then I ship the order
",
        );
        let p = check(&[lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert_eq!(p.len(), 2, "all problems must be reported at once");
    }

    #[test]
    fn checks_background_steps_too() {
        let lf = loaded(
            "Feature: f\n  Background:\n    Given I refund the order\n  Scenario: s\n    Then the response code is 200\n",
        );
        let p = check(&[lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].line, 3);
    }

    #[test]
    fn a_filtered_out_scenario_with_a_bad_step_does_not_fail_validation() {
        let lf = loaded(
            "\
Feature: f
  @smoke
  Scenario: selected
    Then the response code is 200
  @slow
  Scenario: filtered out
    When I refund the order
",
        );

        let p = check(
            &[lf],
            &Registry::new().unwrap(),
            &TagFilter::new(&["smoke".to_string()]),
        );

        assert!(
            p.is_empty(),
            "a typo in an unselected scenario must not fail the run: {p:?}"
        );
    }

    #[test]
    fn checks_expanded_outline_steps() {
        let lf = loaded(
            "\
Feature: f
  Scenario Outline: s
    When I request \"/ping\" using HTTP <method>
    Examples:
      | method |
      | POSTT  |
",
        );
        let p = check(&[lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert_eq!(p.len(), 1, "a typo in the method must surface before the run starts");
    }

    #[test]
    fn steps_with_variables_validate_without_values() {
        // Substitution happens in the arguments, so matching does not require values.
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I request \"/users/<<userId>>\" using HTTP GET\n",
        );
        let p = check(&[lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn macro_call_with_docstring_is_rejected_before_running() {
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I do business\n      \"\"\"\n      x\n      \"\"\"\n",
        );

        let problems = check(&[lf], &macro_registry("docstring"), &TagFilter::new(&[]));

        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("docstring"), "{problems:?}");
    }

    #[test]
    fn macro_call_with_table_is_rejected_before_running() {
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I do business\n      | value |\n      | x     |\n",
        );

        let problems = check(&[lf], &macro_registry("table"), &TagFilter::new(&[]));

        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("table"), "{problems:?}");
    }
}
