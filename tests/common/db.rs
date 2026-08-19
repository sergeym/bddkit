//! DB integration test helpers: recreating the fixture schema and running
//! the compiled binary against a temporary config (like the M1 acceptance tests).

use sqlx::PgPool;
use std::process::Output;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, MutexGuard};

/// The `apibdd_it` schema is shared by all tests; serialize them so
/// one test's fixture recreation doesn't collide with another's.
static DB_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Copy, Default)]
struct TimingRow {
    lock: Duration,
    connect: Duration,
    schema: Duration,
    close: Duration,
    files: Duration,
    bddkit: Duration,
    total: Duration,
}

impl TimingRow {
    fn markdown(self, test: &str) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1_000.0;
        format!(
            "| test | lock | connect | schema | close | files | bddkit | total |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n| {test} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
            ms(self.lock), ms(self.connect), ms(self.schema), ms(self.close),
            ms(self.files), ms(self.bddkit), ms(self.total),
        )
    }
}

fn timings_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

pub struct Setup {
    _guard: MutexGuard<'static, ()>,
    started: Instant,
    timings: TimingRow,
}

pub fn test_dsn() -> String {
    std::env::var("BDDKIT_TEST_DSN")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/postgres".to_string())
}

/// Recreates the `apibdd_it` schema with tables for every PK case.
/// Returns a guard: hold it until the end of the test for isolation.
pub async fn setup() -> Setup {
    let started = Instant::now();
    let phase = Instant::now();
    let guard = DB_LOCK.lock().await;
    let lock = phase.elapsed();
    let phase = Instant::now();
    let pool = PgPool::connect(&test_dsn())
        .await
        .expect("no connection to the test DB; run `docker compose up -d`");
    let connect = phase.elapsed();
    let phase = Instant::now();
    sqlx::query("DROP SCHEMA IF EXISTS apibdd_it CASCADE").execute(&pool).await.unwrap();
    sqlx::query("CREATE SCHEMA apibdd_it").execute(&pool).await.unwrap();
    for stmt in [
        "CREATE TABLE apibdd_it.companies (id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, \
         slug text NOT NULL, name text, created_at timestamptz NOT NULL DEFAULT now())",
        "CREATE TABLE apibdd_it.users (id uuid PRIMARY KEY, email text NOT NULL, \
         name text, created_at timestamptz NOT NULL, deleted_at timestamptz)",
        "CREATE TABLE apibdd_it.pair (a int NOT NULL, b int NOT NULL, note text, PRIMARY KEY (a, b))",
        "CREATE SEQUENCE apibdd_it.thing_seq",
    ] {
        sqlx::query(stmt).execute(&pool).await.unwrap();
    }
    let schema = phase.elapsed();
    let phase = Instant::now();
    pool.close().await;
    Setup {
        _guard: guard,
        started,
        timings: TimingRow { lock, connect, schema, close: phase.elapsed(), ..Default::default() },
    }
}

/// Writes a temporary config (`default` connection → test DB, search_path
/// `apibdd_it`) and one feature file, runs the binary, returns its output.
#[track_caller]
pub fn run_feature(feature_src: &str, setup: &Setup) -> Output {
    let phase = Instant::now();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("bddkit-db-{nanos}"));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(dir.join("features/db.feature"), feature_src).expect("write feature");

    let features_path = dir.join("features").display().to_string().replace('\\', "/");
    let cfg = format!(
        "paths: [{features_path}]\nresources:\n  api:\n    stub:\n      base_url: http://127.0.0.1:1\n  \
         db:\n    default:\n      dsn: {}\n      search_path: [apibdd_it]\n",
        test_dsn()
    );
    let cfg_path = dir.join("cfg.yaml");
    std::fs::write(&cfg_path, cfg).expect("write config");

    let files = phase.elapsed();
    let phase = Instant::now();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg_path.to_str().expect("path is UTF-8")])
        .output()
        .expect("failed to run bddkit");
    let timings = TimingRow {
        files,
        bddkit: phase.elapsed(),
        total: setup.started.elapsed(),
        ..setup.timings
    };
    if timings_enabled(std::env::var("BDDKIT_TEST_TIMINGS").ok().as_deref()) {
        let caller = std::panic::Location::caller();
        eprintln!("{}", timings.markdown(&format!("{}:{}", caller.file(), caller.line())));
    }
    out
}

/// stdout+stderr as one string — for checking error messages.
pub fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_row_reports_all_phases() {
        let row = TimingRow::default().markdown("example");
        assert!(
            row.contains("lock")
                && row.contains("connect")
                && row.contains("schema")
                && row.contains("files")
                && row.contains("bddkit")
                && row.contains("total"),
            "{row}"
        );
        assert_eq!(row.lines().count(), 3, "{row}");
    }

    #[test]
    fn timings_are_enabled_only_by_one() {
        assert!(timings_enabled(Some("1")));
        assert!(!timings_enabled(None));
        assert!(!timings_enabled(Some("true")));
    }
}
