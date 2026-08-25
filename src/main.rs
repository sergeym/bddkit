mod config;
mod db;
mod feature;
mod hawk;
mod http;
mod json;
mod macros;
mod options;
mod plugin;
mod polling;
mod report;
mod runner;
mod srp;
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
    /// Override APP_ENV: selects .env.<name> / .env.<name>.local
    #[arg(long = "env")]
    env: Option<String>,
    /// Stop dispatching new files after the first failure
    #[arg(long = "fail-fast")]
    fail_fast: bool,
}

/// Everything that fails before the first request must exit with code 2 (invariant 6):
/// config loading, path traversal, building API resources and DB pools, parsing
/// scheduling tags — this is a "nothing ran" failure, while 1 is reserved for
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

/// Reads the lock file, loads what it names, and resolves each group's
/// default. Every failure here is a "nothing ran" failure: the caller's `?`
/// carries it to `main`, which exits 2.
///
/// `None` means no plugin was installed at all — the path every existing suite
/// takes, and the one that must cost nothing.
fn load_plugins(
    cli: &Cli,
    cfg: &config::Config,
    generator: &unique::Generator,
) -> Result<Option<Arc<plugin::Plugins>>> {
    let groups_in_config: Vec<String> = cfg.group_names().cloned().collect();
    let mut plugins = plugin::Plugins::load(
        // The same anchor `config::load` uses for the `.env` layers: the lock
        // belongs to the suite, not to whatever directory the run started in.
        plugin::lock::load_default(
            cli.config
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
        )?,
        &cfg.plugin_instances,
        &groups_in_config,
    )?;
    cfg.check_group_defaults()?;
    let mut defaults = std::collections::BTreeMap::new();
    for group in &groups_in_config {
        if let Some(name) = cfg.resolve_default_group(group)? {
            defaults.insert(group.clone(), name);
        }
    }
    plugins.set_defaults(defaults);
    plugins
        .set_artifacts_root(std::env::temp_dir().join(format!("bddkit-{}", generator.run_id())));
    if plugins.is_empty() {
        return Ok(None);
    }

    let plugins = Arc::new(plugins);
    // The libraries are deliberately never unloaded. If a plugin registered a
    // thread-local destructor or an `atexit` handler, running it after
    // `dlclose` executes code in an unmapped page; the process is exiting
    // anyway, so leaking one mapping is cheaper than a segfault in someone's
    // CI. Leaking a reference here rather than at the end of the run is what
    // makes that true on every path out of `run` — an early `?`, a
    // `process::exit(2)`, and the happy path alike.
    //
    // This leaks the mapping only. The instances a plugin created are still
    // dropped through FFI by `Plugins::shutdown`, once the worker pool drains.
    std::mem::forget(plugins.clone());
    Ok(Some(plugins))
}

async fn run(cli: Cli) -> Result<i32> {
    let cfg = config::load(&cli.config, cli.env.as_deref())?;
    // Before the plugins: the artifact root is derived from the run id.
    let generator = Arc::new(unique::Generator::new());
    let plugins = load_plugins(&cli, &cfg, &generator)?;

    // `with_macros_and_plugins`, not `with_macros` plus a registration loop:
    // macros are validated after everything is registered, so a macro body may
    // name a plugin step. This also runs before `validate::check` below, which
    // is what keeps invariant 1 — every step of every selected scenario is
    // matched before the first request.
    let plugin_steps = plugins.as_ref().map(|p| p.steps()).unwrap_or_default();
    let plugin_groups = plugins.as_ref().map(|p| p.group_names()).unwrap_or_default();
    let reg = match macros::MacroCatalog::load(&cfg.macro_paths).and_then(|catalog| {
        steps::Registry::with_macros_and_plugins(catalog, &plugin_steps, &plugin_groups)
    }) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("error: 1 problem, run not started\n\n  {error}");
            std::process::exit(2);
        }
    };

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
            http::ApiResource::new(
                &api.base_url,
                api.timeout_secs,
                headers,
                api.effective_options.clone(),
            )?,
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
    let srp = match cfg.resolve_default_srp()? {
        Some(name) => Some(Arc::new(cfg.resources.srp[&name].to_params()?)),
        None => None,
    };

    println!("run {}", generator.run_id());
    let ctx = Arc::new(runner::RunContext::new(
        reg,
        apis,
        generator.clone(),
        filter,
        db,
        default_db,
        srp,
        plugins.clone(),
        cfg.effective_options.clone(),
        cli.fail_fast,
    ));

    let results = runner::run_all(chains, ctx, cfg.concurrency).await;

    // After the pool has drained, including a failed or --fail-fast run: an
    // instance that outlives the run is a bug the host must not permit. The
    // libraries themselves stay mapped — see `load_plugins`.
    if let Some(plugins) = &plugins {
        plugins.shutdown();
    }

    Ok(report::print_summary(&results, generator.run_id()))
}
