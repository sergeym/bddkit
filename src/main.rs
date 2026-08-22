mod config;
mod db;
mod feature;
mod http;
mod json;
mod macros;
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
    /// Run only these directories or .feature files instead of `paths` from the config
    paths: Vec<PathBuf>,
    /// Run only scenarios with one of these tags (repeatable)
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Override APP_ENV: selects .env.<name> / .env.<name>.local
    #[arg(long = "env")]
    env: Option<String>,
    /// Stop handing out new files after the first failure
    #[arg(long = "fail-fast")]
    fail_fast: bool,
}

/// Anything that fails before the first request must exit with code 2 (invariant 6):
/// config loading, path traversal, building API resources and DB pools, parsing
/// scheduling tags — this is a "nothing ever ran" failure, and 1 is reserved for
/// a failed scenario.
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("error: {error:#}\n\nrun not started");
            std::process::exit(2);
        }
    }
}

async fn run(cli: Cli) -> Result<i32> {
    let cfg = config::load(&cli.config, cli.env.as_deref())?;
    let reg =
        match macros::MacroCatalog::load(&cfg.macro_paths).and_then(steps::Registry::with_macros) {
            Ok(registry) => registry,
            Err(error) => {
                eprintln!("error: 1 problem, run not started\n\n  {error}");
                std::process::exit(2);
            }
        };
    let generator = Arc::new(unique::Generator::new());

    let filter = feature::TagFilter::new(&cli.tags);
    let paths = if cli.paths.is_empty() {
        cfg.paths.as_slice()
    } else {
        cli.paths.as_slice()
    };

    let mut loaded = Vec::new();
    for path in feature::discover(paths)? {
        let lf = feature::load(&path)?;
        if lf.has_selected_scenario(&filter) {
            loaded.push(Arc::new(lf));
        }
    }
    if loaded.is_empty() {
        eprintln!("error: no scenario selected, run not started");
        std::process::exit(2);
    }

    let for_check: Vec<&feature::LoadedFeature> = loaded.iter().map(Arc::as_ref).collect();
    let problems = validate::check(&for_check, &reg, &filter);
    if !problems.is_empty() {
        eprintln!("error: {} problem(s), run not started\n", problems.len());
        for p in &problems {
            eprintln!("{p}");
        }
        std::process::exit(2);
    }

    let chains = runner::build_chains(loaded).map_err(anyhow::Error::msg)?;

    let mut by_name = std::collections::HashMap::new();
    for (name, api) in &cfg.resources.api {
        let headers = api
            .default_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        by_name.insert(
            name.clone(),
            http::ApiResource::new(&api.base_url, api.timeout_secs, headers)?,
        );
    }
    let apis = Arc::new(http::Apis::new(by_name, cfg.resolve_default_api()?)?);

    // Pools are created once per run. The runner is sequential;
    // max_connections is taken with headroom for M5.
    let db = if cfg.resources.db.is_empty() {
        None
    } else {
        Some(Arc::new(
            db::Db::connect(&cfg.resources.db, cfg.concurrency as u32)
                .await
                .map_err(anyhow::Error::msg)?,
        ))
    };
    let default_db = cfg.resolve_default_db()?.unwrap_or_default();

    println!("run {}", generator.run_id());
    let ctx = Arc::new(runner::RunContext::new(
        reg,
        apis,
        generator.clone(),
        filter,
        db,
        default_db,
        cli.fail_fast,
    ));

    let results = runner::run_all(chains, ctx, cfg.concurrency).await;

    Ok(report::print_summary(&results, generator.run_id()))
}
