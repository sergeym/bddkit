mod config;
mod db;
mod feature;
mod http;
mod json;
mod report;
mod runner;
mod steps;
mod unique;
mod validate;
mod vars;
mod world;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "bddkit", about = "Run Gherkin scenarios against an HTTP API")]
struct Cli {
    /// Path to the YAML config
    #[arg(long)]
    config: PathBuf,
    /// Run only one suite
    #[arg(long)]
    suite: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::load(&cli.config)?;
    let reg = steps::Registry::new().map_err(anyhow::Error::msg)?;
    let generator = Arc::new(unique::Generator::new());

    let mut loaded = Vec::new();
    let mut plan = Vec::new();
    for (name, suite) in &cfg.suites {
        if cli.suite.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        for path in feature::discover(&suite.paths)? {
            let lf = feature::load(&path)?;
            plan.push((name.clone(), loaded.len()));
            loaded.push(lf);
        }
    }

    let problems = validate::check(&loaded, &reg);
    if !problems.is_empty() {
        eprintln!("error: {} problem(s), run not started\n", problems.len());
        for p in &problems {
            eprintln!("{p}");
        }
        std::process::exit(2);
    }

    println!("run {}", generator.run_id());
    let mut results = Vec::new();
    let mut infra_failed = false;
    for (suite_name, idx) in plan {
        let suite = &cfg.suites[&suite_name];
        match runner::run_file(
            &loaded[idx],
            &reg,
            &suite.base_url,
            suite.timeout_secs,
            generator.clone(),
        )
        .await
        {
            Ok(r) => {
                report::print_file(&r);
                results.push(r);
            }
            // A file-level infrastructure error (e.g. an invalid base_url)
            // must not abort the whole run: print it and move on to the next file.
            Err(e) => {
                eprintln!("ERROR  {}: {e:#}", loaded[idx].path.display());
                infra_failed = true;
            }
        }
    }

    let code = report::print_summary(&results, generator.run_id());
    std::process::exit(if infra_failed { code.max(1) } else { code });
}
