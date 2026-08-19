use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn default_concurrency() -> usize {
    8
}
fn default_timeout() -> u64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub macro_paths: Vec<PathBuf>,
    pub paths: Vec<PathBuf>,
    pub resources: Resources,
    #[serde(default)]
    pub default_api: Option<String>,
    #[serde(default)]
    pub default_db: Option<String>,
}

/// Resources under test. Not tied to a test set: any scenario can
/// reach any of them by name.
#[derive(Debug, Deserialize)]
pub struct Resources {
    pub api: BTreeMap<String, ApiConfig>,
    #[serde(default)]
    pub db: BTreeMap<String, Connection>,
}

#[derive(Debug, Deserialize)]
pub struct ApiConfig {
    pub base_url: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub default_headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct Connection {
    pub dsn: String,
    #[serde(default)]
    pub search_path: Vec<String>,
}

impl Config {
    /// Default API name: the explicit `default_api`, or the sole resource.
    pub fn resolve_default_api(&self) -> Result<String> {
        resolve_default(
            "default_api",
            "resources.api",
            &self.default_api,
            self.resources.api.keys(),
        )?
        .ok_or_else(|| {
            anyhow::anyhow!("no API resource declared in the config (resources.api)")
        })
    }

    /// Default connection name. `None` if no DBs are declared at all —
    /// that's legal, DB steps then fail on the first reference.
    pub fn resolve_default_db(&self) -> Result<Option<String>> {
        resolve_default(
            "default_db",
            "resources.db",
            &self.default_db,
            self.resources.db.keys(),
        )
    }
}

/// Shared logic for api and db: an explicit name is checked for existence,
/// a sole resource becomes the default itself, several without an explicit one is an error.
fn resolve_default<'a>(
    field: &str,
    section: &str,
    explicit: &Option<String>,
    mut names: impl Iterator<Item = &'a String> + Clone,
) -> Result<Option<String>> {
    if let Some(name) = explicit {
        if names.clone().any(|n| n == name) {
            return Ok(Some(name.clone()));
        }
        bail!("{field} refers to an undeclared resource {name:?} (missing from {section})");
    }
    let first = names.next();
    match (first, names.next()) {
        (None, _) => Ok(None),
        (Some(only), None) => Ok(Some(only.clone())),
        (Some(_), Some(_)) => bail!(
            "{field} is not set, and {section} declares several resources — specify which one is the default"
        ),
    }
}

/// Expands `${VAR}` from the environment. An unknown variable is an error,
/// not an empty string: silently hitting the DB without a password is worse than not starting.
fn expand_env(src: &str) -> Result<String> {
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("constant regex");
    let mut out = String::with_capacity(src.len());
    let mut last = 0usize;
    for c in re.captures_iter(src) {
        let m = c.get(0).expect("group 0 always exists");
        out.push_str(&src[last..m.start()]);
        let name = c.get(1).expect("group 1 is required").as_str();
        match std::env::var(name) {
            Ok(v) => out.push_str(&v),
            Err(_) => bail!("environment variable {name} is not set but is referenced in the config"),
        }
        last = m.end();
    }
    out.push_str(&src[last..]);
    Ok(out)
}

pub fn load(path: &Path) -> Result<Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let expanded = expand_env(&raw)?;
    let cfg: Config = serde_yaml_ng::from_str(&expanded).with_context(|| {
        format!(
            "failed to parse config {}. Format changed: suites was replaced by \
             paths + resources, see docs/writing-tests.md",
            path.display()
        )
    })?;
    // Resolve defaults right away: an ambiguous config must fail at startup,
    // not at the first step that reaches for them.
    cfg.resolve_default_api()?;
    cfg.resolve_default_db()?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
concurrency: 8
paths: [features]
default_api: review
resources:
  api:
    review:
      base_url: http://review.local
    billing:
      base_url: http://billing.local
      timeout_secs: 5
";

    fn parse(src: &str) -> Result<Config> {
        let expanded = expand_env(src)?;
        Ok(serde_yaml_ng::from_str(&expanded)?)
    }

    #[test]
    fn reads_api_resources() {
        let c = parse(SAMPLE).expect("config parses");
        assert_eq!(c.resources.api["review"].base_url, "http://review.local");
    }

    #[test]
    fn timeout_defaults_to_twenty_seconds() {
        let c = parse(SAMPLE).expect("config parses");
        assert_eq!(c.resources.api["review"].timeout_secs, 20);
    }

    #[test]
    fn timeout_can_be_set_per_api_resource() {
        let c = parse(SAMPLE).expect("config parses");
        assert_eq!(c.resources.api["billing"].timeout_secs, 5);
    }

    #[test]
    fn concurrency_defaults_to_eight() {
        let c = parse("paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n")
            .expect("config parses");
        assert_eq!(c.concurrency, 8);
    }

    #[test]
    fn expands_environment_variables() {
        unsafe { std::env::set_var("BDDKIT_TEST_HOST", "example.test") };
        let c = parse(
            "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://${BDDKIT_TEST_HOST}\n",
        )
        .expect("config parses");
        assert_eq!(c.resources.api["a"].base_url, "http://example.test");
    }

    #[test]
    fn missing_environment_variable_is_an_error() {
        let err = parse("paths: [f]\nresources:\n  api:\n    a:\n      base_url: ${BDDKIT_ABSENT_VAR}\n")
            .expect_err("variable is missing");
        assert!(err.to_string().contains("BDDKIT_ABSENT_VAR"), "{err}");
    }

    mod resolve_default_api {
        use super::*;

        #[test]
        fn a_single_resource_needs_no_explicit_default() {
            let c = parse("paths: [f]\nresources:\n  api:\n    only:\n      base_url: http://a.local\n")
                .expect("config parses");
            assert_eq!(c.resolve_default_api().expect("default is inferred"), "only");
        }

        #[test]
        fn an_explicit_default_wins() {
            let c = parse(SAMPLE).expect("config parses");
            assert_eq!(c.resolve_default_api().expect("default is set"), "review");
        }

        #[test]
        fn several_resources_without_a_default_is_an_error() {
            let src = "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n    \
                       b:\n      base_url: http://b.local\n";
            let c = parse(src).expect("config parses");
            let err = c.resolve_default_api().expect_err("default is ambiguous");
            assert!(err.to_string().contains("default_api"), "{err}");
        }

        #[test]
        fn a_default_naming_an_undeclared_resource_is_an_error() {
            let src = "paths: [f]\ndefault_api: missing\nresources:\n  api:\n    a:\n      base_url: http://a.local\n";
            let c = parse(src).expect("config parses");
            let err = c.resolve_default_api().expect_err("resource is not declared");
            assert!(err.to_string().contains("missing"), "{err}");
        }

        #[test]
        fn an_empty_api_map_is_an_error() {
            let c = parse("paths: [f]\nresources:\n  api: {}\n").expect("config parses");
            let err = c.resolve_default_api().expect_err("no resources");
            assert!(err.to_string().contains("resources.api"), "{err}");
        }
    }

    mod resolve_default_db {
        use super::*;

        #[test]
        fn no_databases_resolves_to_none() {
            let c = parse(SAMPLE).expect("config parses");
            assert_eq!(c.resolve_default_db().expect("no DBs declared"), None);
        }

        #[test]
        fn a_single_connection_needs_no_explicit_default() {
            let src = "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                       db:\n    only:\n      dsn: postgres://u:p@db/x\n";
            let c = parse(src).expect("config parses");
            assert_eq!(
                c.resolve_default_db().expect("default is inferred"),
                Some("only".to_string())
            );
        }

        #[test]
        fn several_connections_without_a_default_is_an_error() {
            let src = "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                       db:\n    one:\n      dsn: postgres://u:p@db/x\n    two:\n      dsn: postgres://u:p@db/y\n";
            let c = parse(src).expect("config parses");
            let err = c.resolve_default_db().expect_err("default is ambiguous");
            assert!(err.to_string().contains("default_db"), "{err}");
        }

        #[test]
        fn reads_search_path() {
            let src = "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                       db:\n    main:\n      dsn: postgres://u:p@db/x\n      search_path: [app, public]\n";
            let c = parse(src).expect("config parses");
            assert_eq!(c.resources.db["main"].search_path, vec!["app", "public"]);
        }

        #[test]
        fn search_path_defaults_to_empty() {
            let src = "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                       db:\n    main:\n      dsn: postgres://u:p@db/x\n";
            let c = parse(src).expect("config parses");
            assert!(c.resources.db["main"].search_path.is_empty());
        }
    }

    #[test]
    fn load_reads_a_config_file() {
        let path = std::env::temp_dir().join("bddkit_cfg_load_reads_a_config_file.yaml");
        std::fs::write(
            &path,
            "paths: [features]\nresources:\n  api:\n    review:\n      base_url: http://review.local\n",
        )
        .expect("writing the config");
        let c = load(&path).expect("config loads");
        assert_eq!(c.paths, vec![PathBuf::from("features")]);
    }

    #[test]
    fn load_rejects_an_ambiguous_default_api() {
        let path = std::env::temp_dir().join("bddkit_cfg_load_rejects_ambiguous.yaml");
        std::fs::write(
            &path,
            "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n    b:\n      base_url: http://b.local\n",
        )
        .expect("writing the config");
        let err = load(&path).expect_err("default is ambiguous");
        assert!(err.to_string().contains("default_api"), "{err}");
    }
}
