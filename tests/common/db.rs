//! Helpers for DB integration tests: recreating the fixture schema and running
//! the compiled binary against a temp config (like the M1 acceptance tests).

use sqlx::PgPool;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, MutexGuard};

/// The `apibdd_it` schema is shared by all tests; we serialize them so
/// recreating the fixture in one test doesn't collide with another.
static DB_LOCK: Mutex<()> = Mutex::const_new(());

pub fn test_dsn() -> String {
    std::env::var("BDDKIT_TEST_DSN")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string())
}

/// Recreates the `apibdd_it` schema with tables covering every PK case.
/// Returns a guard: hold it until the end of the test for isolation.
pub async fn setup() -> MutexGuard<'static, ()> {
    let guard = DB_LOCK.lock().await;
    let pool = PgPool::connect(&test_dsn())
        .await
        .expect("no connection to the test DB; run `docker compose up -d`");
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
    pool.close().await;
    guard
}

/// Writes a temp config (connection `default` → test DB, search_path
/// `apibdd_it`) and one feature file, runs the binary, returns its output.
pub fn run_feature(feature_src: &str) -> Output {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("bddkit-db-{nanos}"));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(dir.join("features/db.feature"), feature_src).expect("write feature");

    let features_path = dir.join("features").display().to_string().replace('\\', "/");
    let cfg = format!(
        "suites:\n  s:\n    base_url: http://127.0.0.1:1\n    paths: [{features_path}]\n    \
         connections:\n      default:\n        dsn: {}\n        search_path: [apibdd_it]\n",
        test_dsn()
    );
    let cfg_path = dir.join("cfg.yaml");
    std::fs::write(&cfg_path, cfg).expect("write config");

    std::process::Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg_path.to_str().expect("path is UTF-8")])
        .output()
        .expect("failed to run bddkit")
}

/// stdout+stderr combined into one string — for checking error messages.
pub fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
