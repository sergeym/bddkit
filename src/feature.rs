use anyhow::{Context, Result};
use gherkin::{Feature, GherkinEnv};

/// Parses Gherkin from a string. Used by tests; `parse_path` reads files.
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
}
