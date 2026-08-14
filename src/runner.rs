use crate::feature::{ExpandedStep, LoadedFeature, expand_outlines};
use crate::report::{FileResult, ScenarioResult};
use crate::steps::{Args, Registry, dispatch};
use crate::unique::Generator;
use crate::vars::{VarStack, interpolate};
use crate::world::World;
use anyhow::Result;
use std::sync::Arc;

fn prepare(
    step: &ExpandedStep,
    caps: Vec<String>,
    vars: &VarStack,
    generator: &Generator,
) -> Result<Args, String> {
    // Substitution is applied to arguments, doc strings, and table cells —
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

/// Runs one feature file. The variable frame is shared for the file; HTTP state
/// is recreated for every scenario; Background reruns before each one.
pub async fn run_file(
    lf: &LoadedFeature,
    reg: &Registry,
    base_url: &str,
    timeout_secs: u64,
    generator: Arc<Generator>,
) -> Result<FileResult> {
    let mut world = World::new(base_url, timeout_secs, generator)?;
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
    // with the later `&mut world` needed to call dispatch.
    let generator = world.generator.clone();

    for sc in &lf.feature.scenarios {
        for ex in expand_outlines(sc) {
            world.reset_scenario(base_url, timeout_secs)?;
            let mut failure = None;

            for step in background.iter().chain(ex.steps.iter()) {
                let Some((id, caps)) = reg.find(&step.text).map_err(anyhow::Error::msg)? else {
                    failure = Some(format!("  {}\n    unknown step", step.text));
                    break;
                };
                let args = match prepare(step, caps, &world.vars, &generator) {
                    Ok(a) => a,
                    Err(e) => {
                        failure = Some(format!("  {}\n    {e}", step.text));
                        break;
                    }
                };
                if let Err(e) = dispatch(&mut world, id, &args).await {
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
    Ok(FileResult {
        path: lf.path.clone(),
        scenarios,
    })
}
