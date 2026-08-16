pub mod introspect;
pub mod ops;
pub mod plan;
pub mod reference;
pub mod value;

use crate::config::Connection;
use plan::TableSchema;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// The DB resources for one suite: pools keyed by connection name + an introspection cache.
/// Shared across all files in the suite; the cache stays valid for the whole run (schema doesn't change).
pub struct SuiteDb {
    pools: HashMap<String, PgPool>,
    default: String,
    cache: Mutex<HashMap<(String, String), Arc<TableSchema>>>,
}

impl SuiteDb {
    pub async fn connect(conns: &BTreeMap<String, Connection>, max: u32) -> Result<SuiteDb, String> {
        let mut pools = HashMap::new();
        for (name, c) in conns {
            let opts = PgPoolOptions::new().max_connections(max.max(1));
            let pool = if c.search_path.is_empty() {
                opts.connect(&c.dsn).await
            } else {
                let stmt = format!("SET search_path TO {}", c.search_path.join(", "));
                opts.after_connect(move |conn, _meta| {
                    let stmt = stmt.clone();
                    Box::pin(async move {
                        sqlx::query(&stmt).execute(conn).await.map(|_| ())
                    })
                })
                .connect(&c.dsn)
                .await
            };
            let pool = pool.map_err(|e| format!("connection {name}: {e}"))?;
            pools.insert(name.clone(), pool);
        }
        let default = conns.keys().next().cloned().unwrap_or_default();
        Ok(SuiteDb { pools, default, cache: Mutex::new(HashMap::new()) })
    }

    pub fn pool(&self, name: &str) -> Result<&PgPool, String> {
        self.pools
            .get(name)
            .ok_or_else(|| format!("connection {name:?} is not configured in the suite"))
    }

    /// Introspection with caching. The std::Mutex is NOT held across an await.
    pub async fn schema(&self, conn: &str, sql_name: &str) -> Result<Arc<TableSchema>, String> {
        let key = (conn.to_string(), sql_name.to_string());
        if let Some(s) = self.cache.lock().expect("cache mutex").get(&key) {
            return Ok(s.clone());
        }
        let pool = self.pool(conn)?;
        let schema = Arc::new(introspect::introspect(pool, sql_name).await?);
        self.cache.lock().expect("cache mutex").insert(key, schema.clone());
        Ok(schema)
    }
}

/// A DB handle for one file run: a reference to the suite's resources + the current connection.
/// `current` resets at the scenario boundary (§8: connection scope is the scenario).
pub struct DbHandle {
    suite: Option<Arc<SuiteDb>>,
    current: String,
}

impl DbHandle {
    pub fn new(suite: Option<Arc<SuiteDb>>) -> Self {
        let current = suite.as_ref().map(|s| s.default.clone()).unwrap_or_default();
        Self { suite, current }
    }

    pub fn reset(&mut self) {
        if let Some(s) = &self.suite {
            self.current = s.default.clone();
        }
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn suite(&self) -> Result<&Arc<SuiteDb>, String> {
        self.suite
            .as_ref()
            .ok_or_else(|| "this suite has no database connections configured".to_string())
    }

    pub fn set_current(&mut self, name: &str) -> Result<(), String> {
        self.suite()?.pool(name)?; // check that the connection exists
        self.current = name.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_without_suite_reports_no_db() {
        let mut h = DbHandle::new(None);
        assert!(h.suite().is_err(), "suite() is an error with no connections");
        assert!(h.set_current("x").is_err(), "switching with no suite is an error");
        assert_eq!(h.current(), "", "the default connection is empty");
        h.reset(); // must not panic
    }
}
