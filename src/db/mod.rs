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

/// Pools for every declared connection + an introspection cache. One per run:
/// a connection belongs to the system under test, not to a test suite.
pub struct Db {
    pools: HashMap<String, PgPool>,
    cache: Mutex<HashMap<(String, String), Arc<TableSchema>>>,
}

impl Db {
    pub async fn connect(conns: &BTreeMap<String, Connection>, max: u32) -> Result<Db, String> {
        let mut pools = HashMap::new();
        for (name, c) in conns {
            let opts = PgPoolOptions::new().max_connections(max.max(1));
            let pool = if c.search_path.is_empty() {
                opts.connect(&c.dsn).await
            } else {
                let stmt = format!("SET search_path TO {}", c.search_path.join(", "));
                opts.after_connect(move |conn, _meta| {
                    let stmt = stmt.clone();
                    Box::pin(async move { sqlx::query(&stmt).execute(conn).await.map(|_| ()) })
                })
                .connect(&c.dsn)
                .await
            };
            let pool = pool.map_err(|e| format!("connection {name}: {e}"))?;
            pools.insert(name.clone(), pool);
        }
        Ok(Db {
            pools,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn pool(&self, name: &str) -> Result<&PgPool, String> {
        self.pools
            .get(name)
            .ok_or_else(|| format!("connection {name:?} is not declared in resources.db"))
    }

    /// Cached introspection. std::Mutex is NEVER held across an await.
    pub async fn schema(&self, conn: &str, sql_name: &str) -> Result<Arc<TableSchema>, String> {
        let key = (conn.to_string(), sql_name.to_string());
        if let Some(s) = self.cache.lock().expect("cache mutex").get(&key) {
            return Ok(s.clone());
        }
        let pool = self.pool(conn)?;
        let schema = Arc::new(introspect::introspect(pool, sql_name).await?);
        self.cache
            .lock()
            .expect("cache mutex")
            .insert(key, schema.clone());
        Ok(schema)
    }
}

/// A DB handle for one file run: a reference to the shared pools + the current connection.
/// `current` resets at the scenario boundary (§8: a connection's scope is the scenario).
pub struct DbHandle {
    db: Option<Arc<Db>>,
    default: String,
    current: String,
}

impl DbHandle {
    pub fn new(db: Option<Arc<Db>>, default: String) -> Self {
        Self {
            db,
            current: default.clone(),
            default,
        }
    }

    pub fn reset(&mut self) {
        self.current = self.default.clone();
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn resources(&self) -> Result<&Arc<Db>, String> {
        self.db.as_ref().ok_or_else(|| {
            "no DB connection is declared in the config (resources.db)".to_string()
        })
    }

    pub fn set_current(&mut self, name: &str) -> Result<(), String> {
        self.resources()?.pool(name)?; // check that the connection exists
        self.current = name.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_without_connections_reports_no_database() {
        let h = DbHandle::new(None, String::new());
        assert!(
            h.resources().is_err(),
            "resources() without connections is an error"
        );
    }

    #[test]
    fn a_handle_without_connections_cannot_switch() {
        let mut h = DbHandle::new(None, String::new());
        assert!(
            h.set_current("x").is_err(),
            "switching without connections is an error"
        );
    }

    #[test]
    fn reset_returns_to_the_default_connection() {
        let mut h = DbHandle::new(None, "main".to_string());
        h.current = "audit".to_string();
        h.reset();
        assert_eq!(h.current(), "main");
    }
}
