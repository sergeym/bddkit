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
    pub suites: BTreeMap<String, Suite>,
}

#[derive(Debug, Deserialize)]
pub struct Suite {
    pub base_url: String,
    pub paths: Vec<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub concurrency: Option<usize>,
    #[serde(default)]
    pub connections: BTreeMap<String, Connection>,
}

#[derive(Debug, Deserialize)]
pub struct Connection {
    pub dsn: String,
    #[serde(default)]
    pub search_path: Vec<String>,
}

impl Config {
    /// A suite's own value overrides the global one.
    pub fn suite_concurrency(&self, name: &str) -> usize {
        self.suites
            .get(name)
            .and_then(|s| s.concurrency)
            .unwrap_or(self.concurrency)
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
    let cfg: Config = serde_yaml_ng::from_str(&expanded)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    if cfg.suites.is_empty() {
        bail!("no suites declared in the config");
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
concurrency: 8
suites:
  review:
    base_url: http://review.local
    paths: [features/review]
    concurrency: 4
  billing:
    base_url: http://billing.local
    paths: [features/billing]
";

    fn parse(src: &str) -> Result<Config> {
        let expanded = expand_env(src)?;
        Ok(serde_yaml_ng::from_str(&expanded)?)
    }

    #[test]
    fn reads_suites_and_defaults() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.concurrency, 8);
        assert_eq!(c.suites.len(), 2);
        assert_eq!(c.suites["review"].base_url, "http://review.local");
        assert_eq!(c.suites["billing"].timeout_secs, 20, "default timeout");
    }

    #[test]
    fn suite_concurrency_overrides_global() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(
            c.suite_concurrency("review"),
            4,
            "the suite's value overrides"
        );
        assert_eq!(c.suite_concurrency("billing"), 8, "otherwise the global one");
    }

    #[test]
    fn expands_environment_variables() {
        unsafe { std::env::set_var("BDDKIT_TEST_HOST", "example.test") };
        let c = parse("suites:\n  s:\n    base_url: http://${BDDKIT_TEST_HOST}\n    paths: [f]\n")
            .unwrap();
        assert_eq!(c.suites["s"].base_url, "http://example.test");
    }

    #[test]
    fn missing_environment_variable_is_an_error() {
        let err = parse("suites:\n  s:\n    base_url: ${BDDKIT_ABSENT_VAR}\n    paths: [f]\n")
            .unwrap_err();
        assert!(err.to_string().contains("BDDKIT_ABSENT_VAR"), "{err}");
    }

    // Below: tests that call `load` directly (file → env → yaml → validation),
    // not just the private `parse` that skips reading the file.

    #[test]
    fn load_rejects_empty_suites() {
        let path = std::env::temp_dir().join("bddkit_cfg_load_rejects_empty_suites.yaml");
        std::fs::write(&path, "suites: {}\n").unwrap();
        let err = load(&path).unwrap_err();
        assert!(err.to_string().contains("suite"), "{err}");
    }

    #[test]
    fn load_reads_suite_from_file() {
        let path = std::env::temp_dir().join("bddkit_cfg_load_reads_suite_from_file.yaml");
        std::fs::write(
            &path,
            "suites:\n  review:\n    base_url: http://review.local\n    paths: [features/review]\n",
        )
        .unwrap();
        let c = load(&path).unwrap();
        assert_eq!(c.suites.len(), 1);
        assert_eq!(c.suites["review"].base_url, "http://review.local");
        assert_eq!(
            c.suites["review"].paths,
            vec![PathBuf::from("features/review")]
        );
    }

    #[test]
    fn concurrency_defaults_to_eight_when_omitted() {
        let path = std::env::temp_dir()
            .join("bddkit_cfg_concurrency_defaults_to_eight_when_omitted.yaml");
        std::fs::write(
            &path,
            "suites:\n  s:\n    base_url: http://s.local\n    paths: [f]\n",
        )
        .unwrap();
        let c = load(&path).unwrap();
        assert_eq!(c.concurrency, 8, "default global concurrency");
    }

    #[test]
    fn reads_connections_with_search_path() {
        let src = "\
suites:
  review:
    base_url: http://review.local
    paths: [features/review]
    connections:
      default:
        dsn: postgres://u:p@db:5432/review
        search_path: [app, public]
      audit:
        dsn: postgres://u:p@db2:5432/audit
";
        let c = parse(src).unwrap();
        let conns = &c.suites["review"].connections;
        assert_eq!(conns.len(), 2);
        assert_eq!(conns["default"].dsn, "postgres://u:p@db:5432/review");
        assert_eq!(conns["default"].search_path, vec!["app", "public"]);
        assert!(conns["audit"].search_path.is_empty(), "search_path defaults to empty");
    }

    #[test]
    fn suite_without_connections_parses() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.suites["review"].connections.is_empty(), "connections are optional");
    }
}
