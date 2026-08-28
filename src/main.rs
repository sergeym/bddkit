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
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "bddkit",
    version,
    about = "Run Gherkin scenarios against an HTTP API"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the feature files the config selects
    Run(RunArgs),
    /// Show the steps this binary understands
    Steps(StepsArgs),
}

#[derive(Args)]
struct RunArgs {
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

#[derive(Args)]
#[command(after_help = "Examples:
  bddkit steps list                          every builtin step, grouped by resource
  bddkit steps list db                       only the database steps
  bddkit steps list --filter response -v     narrow, and describe what is left
  bddkit steps list --json                   the same listing, machine-readable
  bddkit steps list --config suite.yaml      also the steps of that suite's plugins")]
struct StepsArgs {
    #[command(subcommand)]
    command: Option<StepsCommand>,
}

#[derive(Subcommand)]
enum StepsCommand {
    /// List the available steps, grouped by resource
    List(ListArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Only this resource: api, db, srp, vars, debug, general, or a plugin group
    resource: Option<String>,
    /// Case-insensitive substring match on the step template
    #[arg(long)]
    filter: Option<String>,
    /// Add a one-line description under each step
    #[arg(short, long)]
    verbose: bool,
    /// Machine-readable output
    #[arg(long)]
    json: bool,
    /// Also list the steps of the plugins this config loads
    #[arg(long)]
    config: Option<PathBuf>,
    /// Description language (default: $BDDKIT_LANG, else en)
    #[arg(long)]
    lang: Option<String>,
}

/// Bare `bddkit steps` is a signpost, not an error: it prints what the family
/// can do and how, which is the question someone typing it is asking.
fn steps_command(args: StepsArgs) -> Result<i32> {
    let Some(StepsCommand::List(args)) = args.command else {
        use clap::CommandFactory;
        let mut cli = Cli::command();
        // `build` first: an unbuilt `Command` has not propagated `bin_name` to
        // its subcommands, so the help would print `Usage: steps [COMMAND]` —
        // a line the reader cannot type.
        cli.build();
        cli.find_subcommand_mut("steps")
            .expect("the steps subcommand is declared")
            .print_help()?;
        println!();
        return Ok(0);
    };
    list_steps(args)
}

fn list_steps(args: ListArgs) -> Result<i32> {
    let overlay = steps::help::translations(&steps::help::language(args.lang.as_deref()));
    let mut rows = steps::help::builtin_rows(&overlay);

    // A plugin's vocabulary lives inside its `cdylib`, so listing it means
    // loading it — which is why `--config` is optional here and required by
    // `run`: the common question, "what steps exist", must cost nothing.
    if let Some(path) = &args.config {
        let cfg = config::load(path, None)?;
        let generator = unique::Generator::new();
        if let Some(plugins) = load_plugins(path, &cfg, &generator)? {
            rows.extend(steps::help::plugin_rows(
                plugins.described_steps(),
                &plugins.group_names(),
                &overlay,
            ));
        }
    }

    if let Some(resource) = &args.resource {
        // Checked before filtering, so a typo is named rather than silently
        // producing the same empty output an over-narrow filter does.
        if !rows.iter().any(|row| &row.group == resource) {
            anyhow::bail!("no such resource: {resource:?}");
        }
        rows.retain(|row| &row.group == resource);
    }
    if let Some(filter) = &args.filter {
        // `--json` always emits the description, so there it is searchable
        // whether or not `-v` was passed — `-v` has no other effect on JSON.
        let searches_descriptions = args.verbose || args.json;
        rows.retain(|row| steps::help::matches_filter(row, filter, searches_descriptions));
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        print!("{}", steps::help::render(&rows, args.verbose));
    }
    Ok(0)
}

/// Everything that fails before the first request must exit with code 2 (invariant 6):
/// config loading, path traversal, building API resources and DB pools, parsing
/// scheduling tags — this is a "nothing ran" failure, while 1 is reserved for
/// a failed scenario.
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    // Each command names what did not happen: "run not started" is a lie when
    // the user only asked for a listing.
    let (result, nothing_happened) = match cli.command {
        Command::Run(args) => (run(args).await, "run not started"),
        Command::Steps(args) => (steps_command(args), "nothing listed"),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("error: {error:#}\n\n{nothing_happened}");
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
    config_path: &std::path::Path,
    cfg: &config::Config,
    generator: &unique::Generator,
) -> Result<Option<Arc<plugin::Plugins>>> {
    let groups_in_config: Vec<String> = cfg.group_names().cloned().collect();
    let mut plugins = plugin::Plugins::load(
        // The same anchor `config::load` uses for the `.env` layers: the lock
        // belongs to the suite, not to whatever directory the run started in.
        plugin::lock::load_default(
            config_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
        )?,
        &cfg.plugin_instances,
        &groups_in_config,
        cfg.concurrency,
    )?;
    // Only meaningful once a plugin is loaded. With none there are no resource
    // groups at all, so every top-level `default_*` key is the unknown key
    // `Config` has always tolerated — it has never had `deny_unknown_fields`,
    // and a suite written before plugins existed must keep running unchanged.
    if !plugins.is_empty() {
        cfg.check_group_defaults()?;
    }
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

async fn run(cli: RunArgs) -> Result<i32> {
    let cfg = config::load(&cli.config, cli.env.as_deref())?;
    // Before the plugins: the artifact root is derived from the run id.
    let generator = Arc::new(unique::Generator::new());
    let plugins = load_plugins(&cli.config, &cfg, &generator)?;

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
