use crate::feature::{LoadedFeature, display_path};
use std::path::{Path, PathBuf};

/// Rejects a runtime token or a macro parameter in an include's file
/// literal: the path decides which steps run, so it must be known before
/// interpolation, same reasoning as invariant 1.
#[allow(dead_code)]
pub fn check_literal(path_literal: &str) -> Result<(), String> {
    // Consumed starting in Task 8 when `validate::check` validates include paths
    if path_literal.contains("<<") || path_literal.contains(">>") {
        return Err(format!(
            "I include {path_literal:?}: the path must be a literal — a `<<...>>` \
             runtime token is not allowed, because the path decides which steps \
             run and must be known before the first request"
        ));
    }
    if path_literal.contains('{') || path_literal.contains('}') {
        return Err(format!(
            "I include {path_literal:?}: the path must be a literal — a macro \
             `{{param}}` is not allowed, for the same reason a `<<...>>` token isn't"
        ));
    }
    Ok(())
}

/// Resolves an include's file literal relative to `base_dir` (the directory
/// of the `.feature` file, or the macro YAML file, the step is written in).
/// Absolute literals are used as-is.
#[allow(dead_code)]
pub fn resolve(path_literal: &str, base_dir: &Path) -> Result<PathBuf, String> {
    // Consumed starting in Task 8 when `validate::check` resolves include paths
    let candidate = Path::new(path_literal);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base_dir.join(candidate)
    };
    if !resolved.is_file() {
        return Err(format!(
            "I include {path_literal:?}: no such file ({})",
            display_path(&resolved)
        ));
    }
    if resolved.extension().and_then(|e| e.to_str()) != Some("feature") {
        return Err(format!(
            "I include {path_literal:?}: not a .feature file ({})",
            display_path(&resolved)
        ));
    }
    Ok(resolved)
}

/// Picks the one scenario an include runs. With no name: the file must hold
/// exactly one scenario (an Outline counts as one). With a name: an exact
/// match, and exactly one.
#[allow(dead_code)]
pub fn select_scenario<'a>(
    included: &'a LoadedFeature,
    scenario_name: Option<&str>,
) -> Result<&'a gherkin::Scenario, String> {
    // Consumed starting in Task 8 when `validate::check` and Task 10 when `runner::run_include` select scenarios
    let path = display_path(&included.path);
    match scenario_name {
        None => match included.feature.scenarios.as_slice() {
            [only] => Ok(only),
            scenarios => {
                let names: Vec<&str> = scenarios.iter().map(|s| s.name.as_str()).collect();
                Err(format!(
                    "I include {path:?}: the file must contain exactly one scenario \
                     to be included without naming one, but it has {}: {}",
                    scenarios.len(),
                    names.join(", ")
                ))
            }
        },
        Some(name) => {
            let matches: Vec<&gherkin::Scenario> = included
                .feature
                .scenarios
                .iter()
                .filter(|s| s.name == name)
                .collect();
            match matches.as_slice() {
                [only] => Ok(only),
                [] => Err(format!(
                    "I include {path:?} scenario {name:?}: no scenario with that name"
                )),
                _ => Err(format!(
                    "I include {path:?} scenario {name:?}: more than one scenario has that name"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn check_literal_rejects_a_runtime_token() {
        let err = check_literal("<<name>>.feature").unwrap_err();
        assert!(err.contains("literal"), "{err}");
    }

    #[test]
    fn check_literal_rejects_a_macro_param() {
        let err = check_literal("{name}.feature").unwrap_err();
        assert!(err.contains("literal"), "{err}");
    }

    #[test]
    fn check_literal_accepts_a_plain_path() {
        assert!(check_literal("flows/register.feature").is_ok());
    }

    #[test]
    fn resolve_joins_a_relative_path_to_the_base_dir() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("flows")).unwrap();
        fs::write(
            dir.path().join("flows/register.feature"),
            "Feature: f\n  Scenario: s\n    Given x\n",
        )
        .unwrap();
        let resolved = resolve("flows/register.feature", dir.path()).unwrap();
        assert_eq!(resolved, dir.path().join("flows/register.feature"));
    }

    #[test]
    fn resolve_rejects_a_missing_file() {
        let dir = tempdir().unwrap();
        let err = resolve("nope.feature", dir.path()).unwrap_err();
        assert!(err.contains("nope.feature"), "{err}");
    }

    #[test]
    fn resolve_rejects_a_non_feature_extension() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "hi").unwrap();
        let err = resolve("notes.txt", dir.path()).unwrap_err();
        assert!(err.contains(".feature"), "{err}");
    }

    fn write_feature(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn select_scenario_requires_exactly_one_when_no_name_given() {
        let dir = tempdir().unwrap();
        let path = write_feature(
            dir.path(),
            "two.feature",
            "Feature: f\n  Scenario: a\n    Given x\n  Scenario: b\n    Given y\n",
        );
        let lf = crate::feature::load(&path).unwrap();
        let err = select_scenario(&lf, None).unwrap_err();
        assert!(err.contains("a") && err.contains("b"), "{err}");
    }

    #[test]
    fn select_scenario_picks_the_one_scenario() {
        let dir = tempdir().unwrap();
        let path = write_feature(
            dir.path(),
            "one.feature",
            "Feature: f\n  Scenario: only\n    Given x\n",
        );
        let lf = crate::feature::load(&path).unwrap();
        let sc = select_scenario(&lf, None).unwrap();
        assert_eq!(sc.name, "only");
    }

    #[test]
    fn select_scenario_matches_by_exact_name() {
        let dir = tempdir().unwrap();
        let path = write_feature(
            dir.path(),
            "two.feature",
            "Feature: f\n  Scenario: a\n    Given x\n  Scenario: b\n    Given y\n",
        );
        let lf = crate::feature::load(&path).unwrap();
        let sc = select_scenario(&lf, Some("b")).unwrap();
        assert_eq!(sc.name, "b");
    }

    #[test]
    fn select_scenario_rejects_an_unknown_name() {
        let dir = tempdir().unwrap();
        let path = write_feature(
            dir.path(),
            "one.feature",
            "Feature: f\n  Scenario: only\n    Given x\n",
        );
        let lf = crate::feature::load(&path).unwrap();
        let err = select_scenario(&lf, Some("nope")).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }
}
