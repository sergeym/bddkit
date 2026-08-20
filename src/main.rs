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
    /// Run only these directories or .feature files instead of the config's `paths`
    paths: Vec<PathBuf>,
    /// Run only scenarios with one of these tags (repeatable)
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::load(&cli.config)?;
    let reg = match macros::MacroCatalog::load(&cfg.macro_paths)
        .and_then(steps::Registry::with_macros)
    {
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
            loaded.push(lf);
        }
    }
    if loaded.is_empty() {
        eprintln!("error: no scenario selected, run not started");
        std::process::exit(2);
    }

    let problems = validate::check(&loaded, &reg, &filter);
    if !problems.is_empty() {
        eprintln!("error: {} problems, run not started\n", problems.len());
        for p in &problems {
            eprintln!("{p}");
        }
        std::process::exit(2);
    }

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
    // max_connections is set with headroom for M5.
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
    let mut results = Vec::new();
    for lf in &loaded {
        let handle = db::DbHandle::new(db.clone(), default_db.clone());
        let r =
            runner::run_file(lf, &reg, apis.clone(), generator.clone(), handle, &filter).await;
        report::print_file(&r);
        results.push(r);
    }

    std::process::exit(report::print_summary(&results, generator.run_id()));
}
