pub mod introspect;
pub mod ops;
pub mod plan;
pub mod platform;
pub mod reference;
pub mod value;

use crate::config::Connection;
use crate::db::platform::{PG, Platform};
use crate::db::reference::TableRef;
use crate::options::Options;
use plan::TableSchema;
use sqlx::AnyPool;
use sqlx::any::{AnyArguments, AnyPoolOptions};
use sqlx::{Any, query::Query};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// Binds text parameters (`Option<String>`). Every value crosses the wire as
/// text; the SQL string itself carries the type, via a Postgres-specific
/// `$N::type` cast (a future MySQL platform builds placeholders with no such
/// cast). Shared by step execution (`ops`) and introspection (`introspect`).
pub fn bind_all<'q>(
    mut q: Query<'q, Any, AnyArguments<'q>>,
    binds: &'q [Option<String>],
) -> Query<'q, Any, AnyArguments<'q>> {
    for b in binds {
        q = q.bind(b);
    }
    q
}

/// Everything one connection needs to run and plan a query.
pub(crate) struct ConnectionState {
    pool: AnyPool,
    platform: &'static dyn Platform,
    options: Options,
}

/// Connections + the introspection cache. One per run: a connection belongs
/// to the system under test, not to a test set.
pub struct Db {
    connections: HashMap<String, ConnectionState>,
    cache: Mutex<HashMap<(String, String), Arc<TableSchema>>>,
}

impl Db {
    pub async fn connect(conns: &BTreeMap<String, Connection>, max: u32) -> Result<Db, String> {
        let mut connections = HashMap::new();
        for (name, c) in conns {
            let opts = AnyPoolOptions::new().max_connections(max.max(1));
            let platform: &'static dyn Platform = &PG; // vendor detection is a later task
            let stmts = platform
                .session_setup(&c.search_path)
                .map_err(|e| format!("connection {name}: {e}"))?;
            let pool = if stmts.is_empty() {
                opts.connect(&c.dsn).await
            } else {
                opts.after_connect(move |conn, _meta| {
                    let stmts = stmts.clone();
                    Box::pin(async move {
                        for stmt in &stmts {
                            sqlx::query(stmt).execute(&mut *conn).await?;
                        }
                        Ok(())
                    })
                })
                .connect(&c.dsn)
                .await
            };
            let pool = pool.map_err(|e| format!("connection {name}: {e}"))?;
            connections.insert(
                name.clone(),
                ConnectionState {
                    pool,
                    platform,
                    options: c.effective_options.clone(),
                },
            );
        }
        Ok(Db {
            connections,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// The single place that knows "not declared" for a connection name.
    pub(crate) fn connection(&self, name: &str) -> Result<&ConnectionState, String> {
        self.connections
            .get(name)
            .ok_or_else(|| format!("connection {name:?} is not declared in resources.db"))
    }

    pub fn options(&self, name: &str) -> Result<&Options, String> {
        Ok(&self.connection(name)?.options)
    }

    /// Introspection with caching. std::Mutex is NOT held across an await.
    pub async fn schema(&self, conn: &str, tref: &TableRef) -> Result<Arc<TableSchema>, String> {
        let key = (conn.to_string(), tref.sql_name());
        if let Some(s) = self.cache.lock().expect("cache mutex").get(&key) {
            return Ok(s.clone());
        }
        let state = self.connection(conn)?;
        let schema = Arc::new(introspect::introspect(&state.pool, state.platform, tref).await?);
        self.cache
            .lock()
            .expect("cache mutex")
            .insert(key, schema.clone());
        Ok(schema)
    }
}

/// A DB handle for one file run: a reference to the shared pools + the current
/// connection. `current` resets at the scenario boundary (§8: connection scope is the scenario).
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
            "no database connection is declared in the config (resources.db)".to_string()
        })
    }

    pub fn set_current(&mut self, name: &str) -> Result<(), String> {
        self.resources()?.connection(name)?; // check that the connection exists
        self.current = name.to_string();
        Ok(())
    }

    pub fn options(&self) -> Result<&Options, String> {
        self.resources()?.options(&self.current)
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
            "resources() is an error without connections"
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

    #[tokio::test]
    async fn an_unknown_scheme_is_refused_by_name() {
        sqlx::any::install_default_drivers();
        let mut conns = BTreeMap::new();
        conns.insert(
            "primary".to_string(),
            Connection {
                dsn: "mssql://x/y".to_string(),
                ..Default::default()
            },
        );
        let err = match Db::connect(&conns, 1).await {
            Ok(_) => panic!("connecting to an unknown scheme should fail"),
            Err(e) => e,
        };
        // A bare substring of "connection" (e.g. "c") would pass on any error
        // at all, since Db::connect wraps every failure with
        // "connection {name}: ...". "primary" pins that the name survives,
        // and "mssql" pins the actual behavior under Any: the scheme is
        // rejected at parse time, not dialed as a hostname and failed on DNS.
        assert!(err.contains("primary"), "{err}");
        assert!(err.contains("mssql"), "{err}");
    }
}
