use anyhow::{Context, Result};
use gherkin::{Feature, GherkinEnv};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExpandedStep {
    pub text: String,
    pub line: usize,
    pub docstring: Option<String>,
    pub table: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone)]
pub struct ExpandedScenario {
    pub name: String,
    pub line: usize,
    pub steps: Vec<ExpandedStep>,
}

pub struct LoadedFeature {
    pub path: PathBuf,
    pub feature: Feature,
}

fn to_step(s: &gherkin::Step) -> ExpandedStep {
    ExpandedStep {
        text: s.value.clone(),
        line: s.position.line,
        docstring: s.docstring.clone(),
        table: s.table.as_ref().map(|t| t.rows.clone()),
    }
}

static PLACEHOLDER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"<<(\w+)>>|<(\w+)>").expect("constant regex")
});

/// Substitutes `<key>` from an Examples row. Single pass, because a naive
/// `replace("<key>", v)` would eat the inner `<key>` inside the runtime token `<<key>>`
/// (in `<<userId>>` the substring `<userId>` starts at position 1) — and `<<…>>` must
/// survive untouched until execution. Double brackets match the first alternative
/// and are returned as-is; single ones are replaced with the column value.
fn substitute(text: &str, keys: &[String], row: &[String]) -> String {
    PLACEHOLDER
        .replace_all(text, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                return format!("<<{}>>", m.as_str()); // runtime token, leave it alone
            }
            let key = caps
                .get(2)
                .expect("second group is present when the first is absent")
                .as_str();
            match keys.iter().position(|k| k == key) {
                Some(i) => row[i].clone(),
                None => caps
                    .get(0)
                    .expect("group 0 always exists")
                    .as_str()
                    .to_string(),
            }
        })
        .into_owned()
}

/// Expands a `Scenario Outline` into separate scenarios. A scenario without `Examples`
/// is returned as-is.
pub fn expand_outlines(sc: &gherkin::Scenario) -> Vec<ExpandedScenario> {
    let base: Vec<ExpandedStep> = sc.steps.iter().map(to_step).collect();
    if sc.examples.is_empty() {
        return vec![ExpandedScenario {
            name: sc.name.clone(),
            line: sc.position.line,
            steps: base,
        }];
    }
    let mut out = Vec::new();
    for ex in &sc.examples {
        let Some(table) = &ex.table else { continue };
        let Some(keys) = table.rows.first() else {
            continue;
        };
        for row in table.rows.iter().skip(1) {
            let steps = base
                .iter()
                .map(|s| ExpandedStep {
                    text: substitute(&s.text, keys, row),
                    line: s.line,
                    docstring: s.docstring.as_ref().map(|d| substitute(d, keys, row)),
                    table: s.table.as_ref().map(|t| {
                        t.iter()
                            .map(|r| r.iter().map(|c| substitute(c, keys, row)).collect())
                            .collect()
                    }),
                })
                .collect();
            out.push(ExpandedScenario {
                name: format!("{} [{}]", sc.name, row.join(", ")),
                line: sc.position.line,
                steps,
            });
        }
    }
    out
}

pub fn load(path: &Path) -> Result<LoadedFeature> {
    let feature = Feature::parse_path(path, GherkinEnv::default())
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(LoadedFeature {
        path: path.to_path_buf(),
        feature,
    })
}

/// Recursively collects `.feature` files from directories; a file path is taken as-is.
pub fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in paths {
        collect(p, &mut out)?;
    }
    out.sort();
    Ok(out)
}

fn collect(p: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if p.is_file() {
        out.push(p.to_path_buf());
        return Ok(());
    }
    if !p.is_dir() {
        anyhow::bail!("path {} does not exist", p.display());
    }
    for entry in
        std::fs::read_dir(p).with_context(|| format!("failed to read {}", p.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "feature") {
            out.push(path);
        }
    }
    Ok(())
}

/// Parses Gherkin from a string. Used by tests; `parse_path` reads files.
#[cfg(test)]
pub fn parse_str(src: &str) -> Result<Feature> {
    Feature::parse(src, GherkinEnv::default()).context("failed to parse Gherkin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_background_scenario_and_steps() {
        let src = "\
Feature: demo
  Background:
    Given the \"Accept\" request header is \"application/json\"
  Scenario: ping
    When I request \"/ping\" using HTTP POST
    Then the response code is 200
";
        let f = parse_str(src).unwrap();
        assert_eq!(f.name, "demo");
        assert_eq!(f.background.as_ref().unwrap().steps.len(), 1);
        assert_eq!(f.scenarios.len(), 1);

        let sc = &f.scenarios[0];
        assert_eq!(sc.name, "ping");
        assert_eq!(sc.steps.len(), 2);
        // value does not include the keyword
        assert_eq!(sc.steps[0].value, "I request \"/ping\" using HTTP POST");
        assert_eq!(sc.steps[0].position.line, 5);
    }

    #[test]
    fn scenario_without_examples_expands_to_one() {
        let f =
            parse_str("Feature: f\n  Scenario: s\n    Then the response code is 200\n").unwrap();
        let e = expand_outlines(&f.scenarios[0]);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "s");
        assert_eq!(e[0].steps[0].text, "the response code is 200");
    }

    #[test]
    fn outline_expands_one_scenario_per_example_row() {
        let src = "\
Feature: f
  Scenario Outline: method not allowed
    When I request \"/ping\" using HTTP <method>
    Then the response code is <code>
    Examples:
      | method | code |
      | POST   | 405  |
      | DELETE | 405  |
";
        let f = parse_str(src).unwrap();
        let e = expand_outlines(&f.scenarios[0]);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].steps[0].text, "I request \"/ping\" using HTTP POST");
        assert_eq!(e[0].steps[1].text, "the response code is 405");
        assert_eq!(e[1].steps[0].text, "I request \"/ping\" using HTTP DELETE");
        assert!(
            e[0].name.contains("POST"),
            "the name must distinguish rows: {}",
            e[0].name
        );
    }

    #[test]
    fn outline_substitutes_inside_docstring() {
        let src = "\
Feature: f
  Scenario Outline: body
    Given the request body is:
      \"\"\"
      {\"email\": \"<email>\"}
      \"\"\"
    Examples:
      | email   |
      | a@b.net |
";
        let f = parse_str(src).unwrap();
        let e = expand_outlines(&f.scenarios[0]);
        assert_eq!(e.len(), 1);
        assert!(
            e[0].steps[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("a@b.net")
        );
    }

    #[test]
    fn step_carries_docstring_and_table() {
        let src = "\
Feature: f
  Scenario: s
    Given the request form parameters are:
      | name  | value |
      | email | a@b   |
";
        let f = parse_str(src).unwrap();
        let e = expand_outlines(&f.scenarios[0]);
        let t = e[0].steps[0].table.as_ref().unwrap();
        assert_eq!(t[0], vec!["name".to_string(), "value".to_string()]);
        assert_eq!(t[1], vec!["email".to_string(), "a@b".to_string()]);
    }

    #[test]
    fn runtime_token_survives_when_column_name_collides() {
        // `<<userId>>` contains the substring `<userId>`; a naive replace would eat it.
        let src = "\
Feature: f
  Scenario Outline: name collision
    When I request \"/users/<<userId>>/<action>\" using HTTP GET
    Examples:
      | userId | action |
      | 42     | ban    |
";
        let f = parse_str(src).unwrap();
        let e = expand_outlines(&f.scenarios[0]);
        assert_eq!(e.len(), 1);
        assert_eq!(
            e[0].steps[0].text,
            "I request \"/users/<<userId>>/ban\" using HTTP GET"
        );
    }

    #[test]
    fn outline_substitutes_inside_table_cells() {
        let src = "\
Feature: f
  Scenario Outline: table
    Given the request form parameters are:
      | name  | value    |
      | email | <email>  |
    Examples:
      | email   |
      | a@b.net |
";
        let f = parse_str(src).unwrap();
        let e = expand_outlines(&f.scenarios[0]);
        assert_eq!(e.len(), 1);
        let t = e[0].steps[0].table.as_ref().unwrap();
        assert_eq!(t[1], vec!["email".to_string(), "a@b.net".to_string()]);
    }
}
