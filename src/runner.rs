use crate::feature::{ExpandedStep, LoadedFeature, expand_outlines};
use crate::report::{FileResult, ScenarioResult};
use crate::steps::{Args, Registry, StepTarget, dispatch};
use crate::unique::Generator;
use crate::vars::{VarStack, interpolate};
use crate::world::World;
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn prepare(
    step: &ExpandedStep,
    caps: Vec<String>,
    vars: &VarStack,
    generator: &Generator,
) -> Result<Args, String> {
    // Substitution applies to arguments, the doc string, and table cells —
    // never to the whole step text.
    let caps = caps
        .iter()
        .map(|c| interpolate(c, vars, generator))
        .collect::<Result<Vec<_>, _>>()?;
    let docstring = step
        .docstring
        .as_ref()
        .map(|d| interpolate(d, vars, generator))
        .transpose()?;
    let table = step
        .table
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    r.iter()
                        .map(|c| interpolate(c, vars, generator))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(Args {
        caps,
        docstring,
        table,
    })
}

fn execute_step<'a>(
    world: &'a mut World,
    reg: &'a Registry,
    step: &'a ExpandedStep,
    generator: &'a Generator,
    depth: usize,
) -> Pin<Box<dyn Future<Output = Result<(), String>> + 'a>> {
    Box::pin(async move {
        let Some((target, caps)) = reg.find(&step.text)? else {
            return Err("unknown step".into());
        };
        match target {
            StepTarget::Builtin(id) => {
                let args = prepare(step, caps, &world.vars, generator)?;
                dispatch(world, id, &args).await
            }
            StepTarget::Macro(index) => {
                if step.docstring.is_some() {
                    return Err("a macro call does not support a docstring".into());
                }
                if step.table.is_some() {
                    return Err("macro calls do not support a table".into());
                }
                if depth >= 16 {
                    return Err("macro nesting exceeds 16".into());
                }

                let definition = reg.macro_def(index);
                let args = prepare(step, caps, &world.vars, generator)?;
                world.vars.push_frame();
                for (name, value) in definition.params.iter().zip(args.caps) {
                    world.vars.set(name, value);
                }

                for body_step in &definition.body {
                    let expanded = ExpandedStep {
                        text: body_step.text.clone(),
                        line: step.line,
                        docstring: body_step.docstring.clone(),
                        table: None,
                    };
                    if let Err(error) =
                        execute_step(world, reg, &expanded, generator, depth + 1).await
                    {
                        world.vars.pop_frame(&[])?;
                        return Err(format!("  {}\n{error}", body_step.text));
                    }
                }
                world.vars.pop_frame(&definition.exports)
            }
        }
    })
}

/// Runs one feature file. The variable frame is shared for the file; HTTP state
/// is recreated for every scenario; Background reruns before each one.
pub async fn run_file(
    lf: &LoadedFeature,
    reg: &Registry,
    apis: Arc<crate::http::Apis>,
    generator: Arc<Generator>,
    db: crate::db::DbHandle,
    filter: &crate::feature::TagFilter,
) -> FileResult {
    let mut world = World::new(apis, generator, db);
    let mut scenarios = Vec::new();

    let background: Vec<ExpandedStep> = lf
        .feature
        .background
        .as_ref()
        .map(|bg| {
            bg.steps
                .iter()
                .map(|s| ExpandedStep {
                    text: s.value.clone(),
                    line: s.position.line,
                    docstring: s.docstring.clone(),
                    table: s.table.as_ref().map(|t| t.rows.clone()),
                })
                .collect()
        })
        .unwrap_or_default();

    // The generator handle is cloned once: inside the loop, `&world.generator` conflicts
    // with the later `&mut world` at the dispatch call.
    let generator = world.generator.clone();

    for sc in &lf.feature.scenarios {
        if !filter.matches(&sc.tags) {
            continue;
        }
        for ex in expand_outlines(sc) {
            world.reset_scenario();
            let mut failure = None;

            for step in background.iter().chain(ex.steps.iter()) {
                if let Err(e) = execute_step(&mut world, reg, step, &generator, 0).await {
                    let mut msg = format!("  {}\n{e}", step.text);
                    if let Some(ex) = world.http.last() {
                        msg.push_str(&format!("\n\n{ex}"));
                    }
                    failure = Some(msg);
                    break;
                }
            }
            scenarios.push(ScenarioResult {
                name: ex.name,
                line: ex.line,
                failure,
            });
        }
    }
    FileResult {
        path: lf.path.clone(),
        scenarios,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;
    use crate::feature::parse_str;
    use crate::macros::MacroCatalog;
    use std::path::PathBuf;

    fn registry(name: &str, source: &str) -> Registry {
        let dir =
            std::env::temp_dir().join(format!("bddkit-runner-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("macros.yaml");
        std::fs::write(&path, source).unwrap();
        Registry::with_macros(MacroCatalog::load(&[path]).unwrap()).unwrap()
    }

    async fn run(feature: &str, registry: &Registry) -> FileResult {
        let loaded = LoadedFeature {
            path: PathBuf::from("macro.feature"),
            feature: parse_str(feature).unwrap(),
        };
        let mut by_name = std::collections::HashMap::new();
        by_name.insert(
            "default".to_string(),
            crate::http::ApiResource::new("http://example.test", 1, Vec::new()).unwrap(),
        );
        let apis = Arc::new(crate::http::Apis::new(by_name, Some("default".to_string())).unwrap());
        run_file(
            &loaded,
            registry,
            apis,
            Arc::new(Generator::new()),
            DbHandle::new(None, String::new()),
            &crate::feature::TagFilter::new(&[]),
        )
        .await
    }

    #[tokio::test]
    async fn macro_interpolates_parameter_and_exports_result() {
        let registry = registry(
            "export",
            r#"
- step: 'I remember "{value}"'
  exports: [result]
  do:
    - set variable "private" to "<<value>>"
    - set variable "result" to "<<private>>"
"#,
        );
        let result = run(
            r#"
Feature: macro
  Scenario: export
    Given set variable "outer" to "saved"
    When I remember "<<outer>>"
    Then variable "result" should be equal to "saved"
"#,
            &registry,
        )
        .await;

        assert!(result.scenarios[0].failure.is_none(), "{:?}", result.scenarios[0].failure);
    }

    #[tokio::test]
    async fn macro_drops_private_variables() {
        let registry = registry(
            "private",
            r#"
- step: I create private state
  do:
    - set variable "private" to "hidden"
"#,
        );
        let result = run(
            r#"
Feature: macro
  Scenario: private
    When I create private state
    Then variable "private" should be equal to "hidden"
"#,
            &registry,
        )
        .await;

        let failure = result.scenarios[0].failure.as_deref().unwrap();
        assert!(failure.contains("private") && failure.contains("is not set"), "{failure}");
    }

    #[tokio::test]
    async fn macro_exports_matching_glob() {
        let registry = registry(
            "glob",
            r#"
- step: I create a row
  exports: [last_insert_id_*]
  do:
    - set variable "last_insert_id_users" to "42"
"#,
        );
        let result = run(
            r#"
Feature: macro
  Scenario: glob
    When I create a row
    Then variable "last_insert_id_users" should be equal to "42"
"#,
            &registry,
        )
        .await;

        assert!(result.scenarios[0].failure.is_none(), "{:?}", result.scenarios[0].failure);
    }

    #[tokio::test]
    async fn nested_macro_exports_through_each_frame() {
        let registry = registry(
            "nested-export",
            r#"
- step: 'I make inner "{value}"'
  exports: [inner]
  do:
    - set variable "inner" to "<<value>>"
- step: 'I make outer "{value}"'
  exports: [result]
  do:
    - I make inner "<<value>>"
    - set variable "result" to "<<inner>>"
"#,
        );
        let result = run(
            r#"
Feature: macro
  Scenario: nested
    When I make outer "done"
    Then variable "result" should be equal to "done"
"#,
            &registry,
        )
        .await;

        assert!(result.scenarios[0].failure.is_none(), "{:?}", result.scenarios[0].failure);
    }

    #[tokio::test]
    async fn missing_declared_export_fails_scenario() {
        let registry = registry(
            "missing-export",
            r#"
- step: I forget the result
  exports: [result]
  do:
    - set variable "private" to "x"
"#,
        );
        let result = run(
            "Feature: macro\n  Scenario: missing\n    When I forget the result\n",
            &registry,
        )
        .await;

        let failure = result.scenarios[0].failure.as_deref().unwrap();
        assert!(failure.contains("result") && failure.contains("is not set"), "{failure}");
    }

    #[tokio::test]
    async fn macro_call_rejects_docstring_argument() {
        let registry = registry(
            "call-docstring",
            "- step: I do business\n  do: [the response code is 200]\n",
        );
        let result = run(
            r#"
Feature: macro
  Scenario: docstring
    When I do business
      """
      unsupported
      """
"#,
            &registry,
        )
        .await;

        let failure = result.scenarios[0].failure.as_deref().unwrap();
        assert!(failure.contains("docstring"), "{failure}");
    }

    #[tokio::test]
    async fn macro_call_rejects_table_argument() {
        let registry = registry(
            "call-table",
            "- step: I do business\n  do: [the response code is 200]\n",
        );
        let result = run(
            r#"
Feature: macro
  Scenario: table
    When I do business
      | value |
      | x     |
"#,
            &registry,
        )
        .await;

        let failure = result.scenarios[0].failure.as_deref().unwrap();
        assert!(failure.contains("table"), "{failure}");
    }
}
