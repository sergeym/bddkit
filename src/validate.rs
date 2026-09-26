use crate::feature::{LoadedFeature, TagFilter, display_path, expand_outlines};
use crate::steps::{Registry, StepTarget};
use std::path::{Path, PathBuf};

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
            display_path(&self.file),
            self.line,
            self.message
        )
    }
}

struct SelectedStep {
    text: String,
    line: usize,
    has_docstring: bool,
    table: Option<Vec<Vec<String>>>,
}

/// Matches every step of every file before the first request. Returns ALL
/// problems at once — a run that fails midway due to a typo costs more
/// than a full check that takes milliseconds.
pub fn check(features: &[&LoadedFeature], reg: &Registry, filter: &TagFilter) -> Vec<Problem> {
    let mut problems = Vec::new();
    for lf in features {
        let base_dir = lf
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        for step in selected_steps(lf, filter) {
            match reg.find(&step.text) {
                Ok(Some((StepTarget::Macro(_), _))) if step.has_docstring => {
                    problems.push(Problem {
                        file: lf.path.clone(),
                        line: step.line,
                        message: "macro calls do not support a docstring".into(),
                    });
                }
                Ok(Some((StepTarget::Macro(_), _))) if step.table.is_some() => {
                    problems.push(Problem {
                        file: lf.path.clone(),
                        line: step.line,
                        message: "macro calls do not support a table".into(),
                    })
                }
                Ok(Some((StepTarget::Builtin { id, .. }, caps)))
                    if matches!(
                        id,
                        crate::steps::StepId::Include | crate::steps::StepId::IncludeScenario
                    ) =>
                {
                    if step.has_docstring {
                        problems.push(Problem {
                            file: lf.path.clone(),
                            line: step.line,
                            message: "I include does not support a docstring".into(),
                        });
                        continue;
                    }
                    let scenario_name = if id == crate::steps::StepId::IncludeScenario {
                        Some(caps[1].as_str())
                    } else {
                        None
                    };
                    if let Err(message) =
                        check_one_include(&caps[0], scenario_name, &base_dir, step.table.as_deref())
                    {
                        problems.push(Problem {
                            file: lf.path.clone(),
                            line: step.line,
                            message,
                        });
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => problems.push(Problem {
                    file: lf.path.clone(),
                    line: step.line,
                    message: format!("unknown step: {}", step.text),
                }),
                Err(e) => problems.push(Problem {
                    file: lf.path.clone(),
                    line: step.line,
                    message: e,
                }),
            }
        }
    }
    problems
}

/// Every step a run would execute from this file. Background is always
/// included: it runs before every selected scenario. Filtered-out scenarios
/// are not — a typo in something that never runs must not fail the run.
fn selected_steps(lf: &LoadedFeature, filter: &TagFilter) -> Vec<SelectedStep> {
    let mut all_steps: Vec<SelectedStep> = Vec::new();
    if let Some(bg) = &lf.feature.background {
        for s in &bg.steps {
            all_steps.push(SelectedStep {
                text: s.value.clone(),
                line: s.position.line,
                has_docstring: s.docstring.is_some(),
                table: s.table.as_ref().map(|t| t.rows.clone()),
            });
        }
    }
    for sc in &lf.feature.scenarios {
        if !filter.matches(&sc.tags) {
            continue;
        }
        for ex in expand_outlines(sc) {
            for st in ex.steps {
                all_steps.push(SelectedStep {
                    text: st.text,
                    line: st.line,
                    has_docstring: st.docstring.is_some(),
                    table: st.table,
                });
            }
        }
    }
    all_steps
}

/// The steps `check` matched, asked a second question: does the resource each
/// one reaches for exist. `unserved` answers `Some(message)` for a step whose
/// group has nothing declared to run on. One finding per file and message —
/// the first such step names the line, every later one would only repeat it.
///
/// Deliberately NOT part of `check`, and so never a reason for `run` to exit
/// 2: a suite may declare an empty group and keep the scenarios using it out
/// with `--tag`, and `run` does not know what will be selected until it is.
/// `doctor` applies no filter, so it is the one caller with an answer.
pub fn unserved_resources(
    features: &[&LoadedFeature],
    reg: &Registry,
    filter: &TagFilter,
    unserved: impl Fn(&StepTarget) -> Option<String>,
) -> Vec<Problem> {
    let mut problems: Vec<Problem> = Vec::new();
    for lf in features {
        for step in selected_steps(lf, filter) {
            let Ok(Some((target, _))) = reg.find(&step.text) else {
                continue;
            };
            let Some(message) = unserved(&target) else {
                continue;
            };
            if problems
                .iter()
                .any(|p| p.file == lf.path && p.message == message)
            {
                continue;
            }
            problems.push(Problem {
                file: lf.path.clone(),
                line: step.line,
                message,
            });
        }
    }
    problems
}

/// One include call, checked in full: literal path, resolution, scenario
/// selection, and its `with:`/Examples shape. Does NOT yet recurse into the
/// selected scenario's own steps — Task 9 adds that, plus cycle detection.
fn check_one_include(
    path_literal: &str,
    scenario_name: Option<&str>,
    base_dir: &Path,
    with_table: Option<&[Vec<String>]>,
) -> Result<(), String> {
    crate::include::check_literal(path_literal)?;
    let resolved = crate::include::resolve(path_literal, base_dir)?;
    let included = crate::feature::load(&resolved).map_err(|e| e.to_string())?;
    let scenario = crate::include::select_scenario(&included, scenario_name)?;
    crate::include::build_scenario(scenario, with_table)?;
    Ok(())
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
        let p = check(&[&lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn reports_unknown_step_with_file_and_line() {
        let lf = loaded("Feature: f\n  Scenario: s\n    When I refund the order\n");
        let p = check(&[&lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
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
        let p = check(&[&lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert_eq!(p.len(), 2, "all problems must be reported at once");
    }

    #[test]
    fn checks_background_steps_too() {
        let lf = loaded(
            "Feature: f\n  Background:\n    Given I refund the order\n  Scenario: s\n    Then the response code is 200\n",
        );
        let p = check(&[&lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
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
  Scenario: filtered_out
    When I refund the order
",
        );

        let p = check(
            &[&lf],
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
        let p = check(&[&lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert_eq!(
            p.len(),
            1,
            "a typo in the method must surface before the run"
        );
    }

    #[test]
    fn steps_with_variables_validate_without_values() {
        // Substitution happens in arguments, so matching does not require values.
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I request \"/users/<<userId>>\" using HTTP GET\n",
        );
        let p = check(&[&lf], &Registry::new().unwrap(), &TagFilter::new(&[]));
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn an_unserved_group_is_reported_once_per_file_at_its_first_step() {
        let lf = loaded(
            "\
Feature: f
  Scenario: s
    When I request \"/ping\" using HTTP GET
    Then the response code is 200
  Scenario: t
    Then the response code is 200
",
        );
        let unserved = |target: &StepTarget| match target {
            StepTarget::Builtin { .. } => Some("api declares none".to_string()),
            _ => None,
        };

        let p = unserved_resources(
            &[&lf],
            &Registry::new().unwrap(),
            &TagFilter::new(&[]),
            unserved,
        );

        assert_eq!(p.len(), 1, "one finding per file and group: {p:?}");
        assert_eq!(p[0].line, 3);
        assert_eq!(p[0].message, "api declares none");
    }

    #[test]
    fn a_served_step_is_not_a_finding() {
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I frobnicate\n    Then the response code is 200\n",
        );

        let p = unserved_resources(
            &[&lf],
            &Registry::new().unwrap(),
            &TagFilter::new(&[]),
            |_| None,
        );

        assert!(
            p.is_empty(),
            "an unknown step is `check`'s finding, not this one: {p:?}"
        );
    }

    #[test]
    fn macro_call_with_docstring_is_rejected_before_running() {
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I do business\n      \"\"\"\n      x\n      \"\"\"\n",
        );

        let problems = check(&[&lf], &macro_registry("docstring"), &TagFilter::new(&[]));

        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("docstring"), "{problems:?}");
    }

    #[test]
    fn macro_call_with_table_is_rejected_before_running() {
        let lf = loaded(
            "Feature: f\n  Scenario: s\n    When I do business\n      | value |\n      | x     |\n",
        );

        let problems = check(&[&lf], &macro_registry("table"), &TagFilter::new(&[]));

        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("table"), "{problems:?}");
    }

    #[test]
    fn include_of_a_missing_file_is_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caller.feature");
        std::fs::write(
            &path,
            "Feature: f\n  Scenario: s\n    Given I include \"nope.feature\"\n",
        )
        .unwrap();
        let lf = crate::feature::load(&path).unwrap();
        let reg = crate::steps::Registry::new().unwrap();
        let filter = crate::feature::TagFilter::new(&[]);
        let problems = check(&[&lf], &reg, &filter);
        assert!(problems.iter().any(|p| p.message.contains("nope.feature")));
    }

    #[test]
    fn include_with_a_runtime_token_path_is_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caller.feature");
        std::fs::write(
            &path,
            "Feature: f\n  Scenario: s\n    Given I include \"<<name>>.feature\"\n",
        )
        .unwrap();
        let lf = crate::feature::load(&path).unwrap();
        let reg = crate::steps::Registry::new().unwrap();
        let filter = crate::feature::TagFilter::new(&[]);
        let problems = check(&[&lf], &reg, &filter);
        assert!(problems.iter().any(|p| p.message.contains("literal")));
    }

    #[test]
    fn include_of_a_valid_one_scenario_file_has_no_problem() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("target.feature"),
            "Feature: f\n  Scenario: only\n    Given I am in debug mode\n",
        )
        .unwrap();
        let path = dir.path().join("caller.feature");
        std::fs::write(
            &path,
            "Feature: f\n  Scenario: s\n    Given I include \"target.feature\"\n",
        )
        .unwrap();
        let lf = crate::feature::load(&path).unwrap();
        let reg = crate::steps::Registry::new().unwrap();
        let filter = crate::feature::TagFilter::new(&[]);
        let problems = check(&[&lf], &reg, &filter);
        assert!(problems.is_empty(), "{problems:?}");
    }
}
