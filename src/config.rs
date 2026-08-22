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

/// Resources under test. Not tied to a test suite: any scenario can
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
    /// Default API name. `None` if no APIs are declared at all — this is
    /// legal, HTTP steps then fail on first use (see `resolve_default_db`).
    pub fn resolve_default_api(&self) -> Result<Option<String>> {
        resolve_default(
            "default_api",
            "resources.api",
            &self.default_api,
            self.resources.api.keys(),
        )
    }

    /// Default connection name. `None` if no DBs are declared at all —
    /// this is legal, DB steps then fail on first use.
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
/// a single resource becomes the default on its own, several without an explicit one is an error.
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
        bail!("{field} refers to an undeclared resource {name:?} (not in {section})");
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

/// Reads one `.env` file into `map`; later keys (from later
/// layers) overwrite earlier ones. A missing file is not an error:
/// in the `.env`/`.env.local`/`.env.$APP_ENV`/`.env.$APP_ENV.local` stack,
/// not all four usually exist.
fn load_env_file(path: &Path, map: &mut BTreeMap<String, String>) -> Result<()> {
    match dotenvy::from_path_iter(path) {
        Ok(iter) => {
            for item in iter {
                let (k, v) = item
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                map.insert(k, v);
            }
            Ok(())
        }
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
    }
}

/// APP_ENV selection priority: an explicit CLI flag > a real process
/// environment variable > the APP_ENV value from the base `.env` > "dev".
/// A real APP_ENV takes priority over `.env`, but NOT over the CLI flag —
/// `--env` exists specifically to override it.
fn resolve_app_env(cli_env: Option<&str>, base_map: &BTreeMap<String, String>) -> String {
    cli_env
        .map(str::to_string)
        .or_else(|| std::env::var("APP_ENV").ok())
        .or_else(|| base_map.get("APP_ENV").cloned())
        .unwrap_or_else(|| "dev".to_string())
}

/// The full stack of `.env` layers, in ascending priority order: a value
/// from a later layer overwrites an earlier one. A skipped file simply
/// adds nothing — see `load_env_file`.
fn load_env_layers(config_dir: &Path, app_env: &str) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    load_env_file(&config_dir.join(".env"), &mut map)?;
    load_env_file(&config_dir.join(".env.local"), &mut map)?;
    load_env_file(&config_dir.join(format!(".env.{app_env}")), &mut map)?;
    load_env_file(&config_dir.join(format!(".env.{app_env}.local")), &mut map)?;
    Ok(map)
}

/// Expands environment variables docker-compose style:
/// `$VAR`/`${VAR}` (error if unset), `${VAR:-def}` (value or
/// def if unset/empty), `${VAR-def}` (def only if unset),
/// `${VAR:?msg}`/`${VAR?msg}` (error with message msg, symmetric with `-`),
/// `${VAR:+alt}`/`${VAR+alt}` (alt if set, symmetric with `-`), `$$`
/// as a literal `$`. Value lookup: the real process environment variable
/// first, then `env_map` (the `.env` layers) — the real one always wins.
///
/// ponytail: `arg` (the default/alt/msg text) is taken as-is, without
/// recursively expanding `${...}` inside it (docker compose can do this,
/// this project's configs never needed it). Upgrade if
/// needed by recursively running `expand_env` over the extracted `arg`
/// before use.
fn expand_env(src: &str, env_map: &BTreeMap<String, String>) -> Result<String> {
    let re = regex::Regex::new(
        r"\$\$|\$\{(?P<name>[A-Za-z_][A-Za-z0-9_]*)(?:(?P<op>:-|-|:\?|\?|:\+|\+)(?P<arg>[^}]*))?\}|\$(?P<bare>[A-Za-z_][A-Za-z0-9_]*)",
    )
    .expect("constant regex");

    let lookup = |name: &str| -> Option<String> {
        std::env::var(name).ok().or_else(|| env_map.get(name).cloned())
    };

    let mut out = String::with_capacity(src.len());
    let mut last = 0usize;
    for c in re.captures_iter(src) {
        let m = c.get(0).expect("group 0 always exists");
        out.push_str(&src[last..m.start()]);
        last = m.end();

        if m.as_str() == "$$" {
            out.push('$');
            continue;
        }

        let name = c
            .name("name")
            .or_else(|| c.name("bare"))
            .expect("either name or bare — otherwise the regex would not have matched")
            .as_str();
        let op = c.name("op").map(|o| o.as_str());
        let arg = c.name("arg").map(|a| a.as_str()).unwrap_or("");
        let value = lookup(name);

        let resolved = match op {
            None => match value {
                Some(v) => v,
                None => bail!("environment variable {name} is not set but is referenced in the config"),
            },
            Some(":-") => match value {
                Some(v) if !v.is_empty() => v,
                _ => arg.to_string(),
            },
            Some("-") => value.unwrap_or_else(|| arg.to_string()),
            Some(":?") => match value {
                Some(v) if !v.is_empty() => v,
                _ => bail!("{}", default_missing_message(name, arg)),
            },
            Some("?") => match value {
                Some(v) => v,
                None => bail!("{}", default_missing_message(name, arg)),
            },
            Some(":+") => match value {
                Some(v) if !v.is_empty() => arg.to_string(),
                _ => String::new(),
            },
            Some("+") => match value {
                Some(_) => arg.to_string(),
                None => String::new(),
            },
            Some(other) => unreachable!("the regex only captures known operators, got {other:?}"),
        };
        out.push_str(&resolved);
    }
    out.push_str(&src[last..]);
    Ok(out)
}

/// Error message for `:?`/`?`: the custom text if given,
/// otherwise the standard wording (symmetric with the message for a bare `${VAR}`).
fn default_missing_message(name: &str, custom: &str) -> String {
    if custom.is_empty() {
        format!("environment variable {name} is not set but is referenced in the config")
    } else {
        custom.to_string()
    }
}

pub fn load(path: &Path, cli_env: Option<&str>) -> Result<Config> {
    // path.parent() returns Some("") for a bare file name (not None) —
    // join() with "" gives a path relative to cwd, which is what works in the common case;
    // unwrap_or here is a guard for a path with no parent at all (root/prefix),
    // not the main mechanism.
    let config_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut base_map = BTreeMap::new();
    load_env_file(&config_dir.join(".env"), &mut base_map)?;
    let app_env = resolve_app_env(cli_env, &base_map);
    let env_map = load_env_layers(config_dir, &app_env)?;

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let expanded = expand_env(&raw, &env_map)?;
    let cfg: Config = serde_yaml_ng::from_str(&expanded).with_context(|| {
        format!(
            "failed to parse config {}. Format changed: suites replaced by \
             paths + resources, see docs/writing-tests.md",
            path.display()
        )
    })?;
    // Resolve the defaults right away: an ambiguous config must fail at startup,
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
        let expanded = expand_env(src, &BTreeMap::new())?;
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

    mod expand_env_operators {
        use super::*;

        fn expand(src: &str, pairs: &[(&str, &str)]) -> Result<String> {
            let map: BTreeMap<String, String> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            expand_env(src, &map)
        }

        #[test]
        fn dollar_dollar_is_a_literal_dollar_sign() {
            assert_eq!(expand("$${NOT_A_VAR}", &[]).expect("parses"), "${NOT_A_VAR}");
        }

        #[test]
        fn bare_dollar_var_without_braces_behaves_like_braced_form() {
            let out = expand("$FOO", &[("FOO", "bar")]).expect("FOO is set");
            assert_eq!(out, "bar");
        }

        #[test]
        fn bare_dollar_var_without_braces_errors_when_unset() {
            let err = expand("$FOO", &[]).expect_err("FOO is not set");
            assert!(err.to_string().contains("FOO"), "{err}");
        }

        #[test]
        fn colon_dash_uses_default_when_unset() {
            assert_eq!(expand("${FOO:-fallback}", &[]).expect("parses"), "fallback");
        }

        #[test]
        fn colon_dash_uses_default_when_set_but_empty() {
            assert_eq!(expand("${FOO:-fallback}", &[("FOO", "")]).expect("parses"), "fallback");
        }

        #[test]
        fn colon_dash_uses_value_when_set_and_nonempty() {
            assert_eq!(expand("${FOO:-fallback}", &[("FOO", "real")]).expect("parses"), "real");
        }

        #[test]
        fn dash_uses_default_only_when_unset() {
            assert_eq!(expand("${FOO-fallback}", &[]).expect("parses"), "fallback");
        }

        #[test]
        fn dash_passes_through_empty_value_unchanged() {
            assert_eq!(expand("${FOO-fallback}", &[("FOO", "")]).expect("parses"), "");
        }

        #[test]
        fn colon_question_errors_when_unset_with_custom_message() {
            let err = expand("${FOO:?custom message}", &[]).expect_err("FOO is not set");
            assert_eq!(err.to_string(), "custom message");
        }

        #[test]
        fn colon_question_errors_when_set_but_empty() {
            let err = expand("${FOO:?custom message}", &[("FOO", "")]).expect_err("FOO is empty");
            assert_eq!(err.to_string(), "custom message");
        }

        #[test]
        fn colon_question_with_empty_message_falls_back_to_default_message() {
            let err = expand("${FOO:?}", &[]).expect_err("FOO is not set");
            assert_eq!(
                err.to_string(),
                "environment variable FOO is not set but is referenced in the config"
            );
        }

        #[test]
        fn colon_question_passes_through_value_when_set_and_nonempty() {
            assert_eq!(expand("${FOO:?msg}", &[("FOO", "real")]).expect("parses"), "real");
        }

        #[test]
        fn question_errors_only_when_unset() {
            let err = expand("${FOO?custom message}", &[]).expect_err("FOO is not set");
            assert_eq!(err.to_string(), "custom message");
        }

        #[test]
        fn question_passes_through_empty_value_without_error() {
            assert_eq!(expand("${FOO?custom message}", &[("FOO", "")]).expect("FOO is set to empty"), "");
        }

        #[test]
        fn colon_plus_uses_alt_when_set_and_nonempty() {
            assert_eq!(expand("${FOO:+alt}", &[("FOO", "real")]).expect("parses"), "alt");
        }

        #[test]
        fn colon_plus_is_empty_when_unset() {
            assert_eq!(expand("${FOO:+alt}", &[]).expect("parses"), "");
        }

        #[test]
        fn colon_plus_is_empty_when_set_but_empty() {
            assert_eq!(expand("${FOO:+alt}", &[("FOO", "")]).expect("parses"), "");
        }

        #[test]
        fn plus_uses_alt_when_set_even_if_empty() {
            assert_eq!(expand("${FOO+alt}", &[("FOO", "")]).expect("parses"), "alt");
        }

        #[test]
        fn plus_is_empty_when_unset() {
            assert_eq!(expand("${FOO+alt}", &[]).expect("parses"), "");
        }

        #[test]
        fn real_process_env_wins_over_env_map() {
            // PATH is guaranteed to be set in any process — no need to mutate
            // the real environment to prove priority: if the map won,
            // the result would match the fake value below.
            let out = expand("${PATH}", &[("PATH", "definitely_not_the_real_path")])
                .expect("PATH is set in the real environment");
            assert_ne!(out, "definitely_not_the_real_path");
        }
    }

    mod resolve_default_api {
        use super::*;

        #[test]
        fn a_single_resource_needs_no_explicit_default() {
            let c = parse("paths: [f]\nresources:\n  api:\n    only:\n      base_url: http://a.local\n")
                .expect("config parses");
            assert_eq!(c.resolve_default_api().expect("default is inferred"), Some("only".to_string()));
        }

        #[test]
        fn an_explicit_default_wins() {
            let c = parse(SAMPLE).expect("config parses");
            assert_eq!(c.resolve_default_api().expect("default is set"), Some("review".to_string()));
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
        fn no_apis_resolves_to_none() {
            // Legal: a config may describe only a DB. HTTP steps then
            // fail on first use — symmetric with resolve_default_db.
            let c = parse("paths: [f]\nresources:\n  api: {}\n").expect("config parses");
            assert_eq!(c.resolve_default_api().expect("no APIs are declared"), None);
        }
    }

    mod resolve_default_db {
        use super::*;

        #[test]
        fn no_databases_resolves_to_none() {
            let c = parse(SAMPLE).expect("config parses");
            assert_eq!(c.resolve_default_db().expect("no DBs are declared"), None);
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

    mod env_layers {
        use super::*;

        fn unique_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("bddkit_env_layers_{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp directory");
            dir
        }

        #[test]
        fn load_env_file_is_a_noop_when_file_is_missing() {
            let dir = unique_dir("missing_file");
            let mut map = BTreeMap::new();
            load_env_file(&dir.join(".env"), &mut map).expect("missing file is not an error");
            assert!(map.is_empty());
        }

        #[test]
        fn load_env_file_reads_key_value_pairs() {
            let dir = unique_dir("basic_read");
            std::fs::write(dir.join(".env"), "FOO=bar\nBAZ=qux\n").expect("write .env");
            let mut map = BTreeMap::new();
            load_env_file(&dir.join(".env"), &mut map).expect(".env is read");
            assert_eq!(map.get("FOO"), Some(&"bar".to_string()));
            assert_eq!(map.get("BAZ"), Some(&"qux".to_string()));
        }

        #[test]
        fn load_env_file_reports_the_path_on_a_malformed_file() {
            let dir = unique_dir("malformed_file");
            let path = dir.join(".env");
            std::fs::write(&path, "FOO=\"unterminated\n").expect("write malformed .env");
            let mut map = BTreeMap::new();
            let err = load_env_file(&path, &mut map).expect_err("malformed .env is an error");
            assert!(
                err.to_string().contains(&path.display().to_string()),
                "{err}"
            );
        }

        #[test]
        fn later_file_overrides_earlier_for_same_key() {
            let dir = unique_dir("override");
            let mut map = BTreeMap::new();
            std::fs::write(dir.join("a"), "FOO=from_a\n").expect("write a");
            std::fs::write(dir.join("b"), "FOO=from_b\n").expect("write b");
            load_env_file(&dir.join("a"), &mut map).expect("a is read");
            load_env_file(&dir.join("b"), &mut map).expect("b is read");
            assert_eq!(map.get("FOO"), Some(&"from_b".to_string()));
        }

        #[test]
        fn load_env_layers_applies_full_precedence_order() {
            let dir = unique_dir("full_precedence");
            std::fs::write(dir.join(".env"), "A=base\nB=base\nC=base\nD=base\n").expect("write .env");
            std::fs::write(dir.join(".env.local"), "B=local\nC=local\nD=local\n").expect("write .env.local");
            std::fs::write(dir.join(".env.dev"), "C=dev\nD=dev\n").expect("write .env.dev");
            std::fs::write(dir.join(".env.dev.local"), "D=dev_local\n").expect("write .env.dev.local");

            let map = load_env_layers(&dir, "dev").expect("layers are read");
            assert_eq!(map.get("A"), Some(&"base".to_string()), "only in .env");
            assert_eq!(map.get("B"), Some(&"local".to_string()), ".env.local overrides .env");
            assert_eq!(map.get("C"), Some(&"dev".to_string()), ".env.dev overrides .env.local");
            assert_eq!(
                map.get("D"),
                Some(&"dev_local".to_string()),
                ".env.dev.local is the most specific, it wins"
            );
        }

        #[test]
        fn load_env_layers_ignores_missing_optional_files() {
            let dir = unique_dir("only_base");
            std::fs::write(dir.join(".env"), "A=base\n").expect("write .env");
            let map = load_env_layers(&dir, "prod").expect("missing .env.local/.env.prod* is not an error");
            assert_eq!(map.get("A"), Some(&"base".to_string()));
            assert_eq!(map.len(), 1);
        }

        #[test]
        fn resolve_app_env_cli_flag_wins() {
            let base = BTreeMap::new();
            let resolved = resolve_app_env(Some("from_cli"), &base);
            assert_eq!(resolved, "from_cli");
        }

        #[test]
        fn resolve_app_env_falls_back_to_base_map_value() {
            let mut base = BTreeMap::new();
            base.insert("APP_ENV".to_string(), "from_dotenv".to_string());
            // cli_env and the real APP_ENV variable are both absent in this test —
            // we treat the real APP_ENV as unset in the CI test environment.
            if std::env::var("APP_ENV").is_err() {
                let resolved = resolve_app_env(None, &base);
                assert_eq!(resolved, "from_dotenv");
            }
        }

        #[test]
        fn resolve_app_env_defaults_to_dev() {
            let base = BTreeMap::new();
            if std::env::var("APP_ENV").is_err() {
                let resolved = resolve_app_env(None, &base);
                assert_eq!(resolved, "dev");
            }
        }

        #[test]
        fn resolve_app_env_real_env_var_wins_over_base_map() {
            // The function specifically reads the real APP_ENV — a test variable
            // will not do, so the test mutates the actual APP_ENV for the duration of its
            // run and clears it at the end so it does not leak into neighboring tests.
            unsafe { std::env::set_var("APP_ENV", "from_real_env") };
            let mut base = BTreeMap::new();
            base.insert("APP_ENV".to_string(), "from_base_map".to_string());
            let resolved = resolve_app_env(None, &base);
            unsafe { std::env::remove_var("APP_ENV") };
            assert_eq!(resolved, "from_real_env");
        }
    }

    #[test]
    fn load_reads_a_config_file() {
        let path = std::env::temp_dir().join("bddkit_cfg_load_reads_a_config_file.yaml");
        std::fs::write(
            &path,
            "paths: [features]\nresources:\n  api:\n    review:\n      base_url: http://review.local\n",
        )
        .expect("write config");
        let c = load(&path, None).expect("config loads");
        assert_eq!(c.paths, vec![PathBuf::from("features")]);
    }

    #[test]
    fn load_rejects_an_ambiguous_default_api() {
        let path = std::env::temp_dir().join("bddkit_cfg_load_rejects_ambiguous.yaml");
        std::fs::write(
            &path,
            "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n    b:\n      base_url: http://b.local\n",
        )
        .expect("write config");
        let err = load(&path, None).expect_err("default is ambiguous");
        assert!(err.to_string().contains("default_api"), "{err}");
    }

    mod load_with_env_files {
        use super::*;

        fn write_config(dir: &Path, yaml: &str) -> PathBuf {
            let path = dir.join("config.yaml");
            std::fs::write(&path, yaml).expect("write config");
            path
        }

        fn unique_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("bddkit_load_with_env_{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp directory");
            dir
        }

        #[test]
        fn substitutes_value_from_base_dotenv_file() {
            let dir = unique_dir("base_dotenv");
            std::fs::write(dir.join(".env"), "DB_PASS=secret\n").expect("write .env");
            let path = write_config(
                &dir,
                "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                 db:\n    main:\n      dsn: postgres://u:${DB_PASS}@db/x\n",
            );
            let c = load(&path, None).expect("config loads with .env");
            assert_eq!(c.resources.db["main"].dsn, "postgres://u:secret@db/x");
        }

        #[test]
        fn cli_env_flag_selects_the_matching_env_specific_file() {
            let dir = unique_dir("cli_env_flag");
            std::fs::write(dir.join(".env"), "DB_PASS=base_secret\n").expect("write .env");
            std::fs::write(dir.join(".env.prod"), "DB_PASS=prod_secret\n").expect("write .env.prod");
            let path = write_config(
                &dir,
                "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                 db:\n    main:\n      dsn: postgres://u:${DB_PASS}@db/x\n",
            );
            let c = load(&path, Some("prod")).expect("config loads with --env=prod");
            assert_eq!(c.resources.db["main"].dsn, "postgres://u:prod_secret@db/x");
        }

        #[test]
        fn without_cli_env_flag_defaults_to_dev_file() {
            let dir = unique_dir("default_dev");
            std::fs::write(dir.join(".env"), "DB_PASS=base_secret\n").expect("write .env");
            std::fs::write(dir.join(".env.dev"), "DB_PASS=dev_secret\n").expect("write .env.dev");
            let path = write_config(
                &dir,
                "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                 db:\n    main:\n      dsn: postgres://u:${DB_PASS}@db/x\n",
            );
            if std::env::var("APP_ENV").is_err() {
                let c = load(&path, None).expect("config loads without --env");
                assert_eq!(c.resources.db["main"].dsn, "postgres://u:dev_secret@db/x");
            }
        }

        #[test]
        fn local_file_overrides_base_env_for_same_key() {
            let dir = unique_dir("local_override");
            std::fs::write(dir.join(".env"), "DB_PASS=base_secret\n").expect("write .env");
            std::fs::write(dir.join(".env.local"), "DB_PASS=local_secret\n").expect("write .env.local");
            let path = write_config(
                &dir,
                "paths: [f]\nresources:\n  api:\n    a:\n      base_url: http://a.local\n  \
                 db:\n    main:\n      dsn: postgres://u:${DB_PASS}@db/x\n",
            );
            let c = load(&path, None).expect("config loads");
            assert_eq!(c.resources.db["main"].dsn, "postgres://u:local_secret@db/x");
        }
    }
}
