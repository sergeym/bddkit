use crate::db::bind_all;
use crate::db::plan::{self, InsertPlan};
use crate::db::platform::Platform;
use crate::db::reference::TableRef;
use crate::db::value;
use crate::world::World;
use sqlx::{PgPool, Postgres, Row, postgres::PgArguments, query::Query};
use std::sync::Arc;

/// Prints the SQL and parameters if debug mode is enabled (§8).
pub fn log_sql(w: &World, sql: &str, binds: &[Option<String>], logs: &[String]) {
    if w.debug {
        eprintln!("SQL: {sql}");
        eprintln!("PARAMETERS: {binds:?}");
        for l in logs {
            eprintln!("  auto: {l}");
        }
    }
}

/// Resolves a reference into (pool, platform, schema, parsed reference). The
/// connection comes from the reference's prefix or the scenario's current connection.
pub async fn resolve<'a>(
    w: &'a World,
    raw_table: &str,
) -> Result<(&'a PgPool, &'static dyn Platform, Arc<plan::TableSchema>, TableRef), String> {
    let tref = TableRef::parse(raw_table)?;
    let conn = tref
        .conn
        .clone()
        .unwrap_or_else(|| w.db.current().to_string());
    let db = w.db.resources()?;
    let state = db.connection(&conn)?;
    let schema = db.schema(&conn, &tref).await?;
    Ok((&state.pool, state.platform, schema, tref))
}

/// Inserts one row; stores PK values in `last_insert_*`.
pub async fn insert(
    w: &mut World,
    raw_table: &str,
    values: &[(String, Option<String>)],
    index: Option<usize>,
) -> Result<(), String> {
    let (pool, platform, schema, tref) = resolve(w, raw_table).await?;
    let InsertPlan {
        sql,
        binds,
        var_names,
        logs,
    } = plan::build_insert(platform, &schema, &tref.sql_name(), &tref.table, values, index)?;
    log_sql(w, &sql, &binds, &logs);

    if var_names.is_empty() {
        bind_all(sqlx::query(&sql), &binds)
            .execute(pool)
            .await
            .map_err(|e| format!("INSERT into {}: {e}", tref.sql_name()))?;
        return Ok(());
    }
    let row = bind_all(sqlx::query(&sql), &binds)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("INSERT into {}: {e}", tref.sql_name()))?;
    // PK values come back as text, per Platform::returning, in var_names order.
    let mut assignments: Vec<(String, String)> = Vec::new();
    for (i, name) in var_names.iter().enumerate() {
        let v: String = row
            .try_get(i)
            .map_err(|e| format!("reading RETURNING: {e}"))?;
        assignments.push((name.clone(), v));
    }
    // The pool borrow has ended — now it's safe to write variables.
    for (name, v) in assignments {
        w.vars.set(&name, v);
    }
    Ok(())
}

/// UPDATE ... SET ... WHERE ...; stores the number of affected rows in `updated_<table>`.
pub async fn update(w: &mut World, raw_table: &str, set: &str, where_: &str) -> Result<(), String> {
    let (pool, platform, schema, tref) = resolve(w, raw_table).await?;
    let set_pairs = value::parse_oneliner(set)?;
    let where_pairs = value::parse_oneliner(where_)?;
    let (sql, binds) =
        plan::build_update(platform, &schema, &tref.sql_name(), &set_pairs, &where_pairs)?;
    log_sql(w, &sql, &binds, &[]);
    let done = bind_all(sqlx::query(&sql), &binds)
        .execute(pool)
        .await
        .map_err(|e| format!("UPDATE {}: {e}", tref.sql_name()))?;
    let table = tref.table.clone();
    w.vars.set(
        &format!("updated_{table}"),
        done.rows_affected().to_string(),
    );
    Ok(())
}

/// DELETE FROM ... WHERE ... (an empty WHERE is rejected by the builder);
/// stores the number of deleted rows in `deleted_<table>`.
pub async fn delete(w: &mut World, raw_table: &str, where_: &str) -> Result<(), String> {
    let (pool, platform, schema, tref) = resolve(w, raw_table).await?;
    let where_pairs = value::parse_oneliner(where_)?;
    let (sql, binds) = plan::build_delete(platform, &schema, &tref.sql_name(), &where_pairs)?;
    log_sql(w, &sql, &binds, &[]);
    let done = bind_all(sqlx::query(&sql), &binds)
        .execute(pool)
        .await
        .map_err(|e| format!("DELETE {}: {e}", tref.sql_name()))?;
    let table = tref.table.clone();
    w.vars.set(
        &format!("deleted_{table}"),
        done.rows_affected().to_string(),
    );
    Ok(())
}

/// Checks row presence with a fresh query on every assertion attempt.
pub async fn exists(
    w: &mut World,
    raw_table: &str,
    where_pairs: &[(String, Option<String>)],
) -> Result<bool, String> {
    let (pool, platform, schema, tref) = resolve(w, raw_table).await?;
    let (sql, binds) = plan::build_exists(platform, &schema, &tref.sql_name(), where_pairs)?;
    log_sql(w, &sql, &binds, &[]);
    Ok(bind_all(sqlx::query(&sql), &binds)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("checking presence in {}: {e}", tref.sql_name()))?
        .is_some())
}

/// Reads one column (cast to text via `Platform::cast_text`) from the first
/// row matching a WHERE clause; stores the value in a variable.
pub async fn extract(
    w: &mut World,
    column: &str,
    raw_table: &str,
    where_str: &str,
    var: &str,
) -> Result<(), String> {
    let (pool, platform, schema, tref) = resolve(w, raw_table).await?;
    if schema.col(column).is_none() {
        return Err(format!(
            "column {column:?} is missing from {}",
            tref.sql_name()
        ));
    }
    let where_pairs = value::parse_oneliner(where_str)?;
    let (where_sql, binds) = plan::build_where(platform, &schema, &where_pairs, 1)?;
    let sql = format!(
        "SELECT {} FROM {} WHERE {where_sql} LIMIT 1",
        platform.cast_text(column),
        tref.sql_name()
    );
    log_sql(w, &sql, &binds, &[]);
    let row = bind_all(sqlx::query(&sql), &binds)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("SELECT {}: {e}", tref.sql_name()))?
        .ok_or_else(|| format!("no row in {} matched the condition", tref.sql_name()))?;
    let v: Option<String> = row
        .try_get(0)
        .map_err(|e| format!("reading value: {e}"))?;
    let value = v.unwrap_or_default();
    w.vars.set(var, value);
    Ok(())
}

/// DELETE FROM ... with no WHERE — a full table clear;
/// stores the number of deleted rows in `deleted_<table>`.
pub async fn delete_all(w: &mut World, raw_table: &str) -> Result<(), String> {
    let (pool, _platform, _schema, tref) = resolve(w, raw_table).await?;
    let sql = plan::build_delete_all(&tref.sql_name());
    log_sql(w, &sql, &[], &[]);
    let done = sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| format!("DELETE ALL {}: {e}", tref.sql_name()))?;
    let table = tref.table.clone();
    w.vars.set(
        &format!("deleted_{table}"),
        done.rows_affected().to_string(),
    );
    Ok(())
}

/// Binds typed arguments for a procedure/function call.
fn bind_args<'q>(
    mut q: Query<'q, Postgres, PgArguments>,
    args: &'q [value::Arg],
) -> Query<'q, Postgres, PgArguments> {
    for a in args {
        q = match a {
            value::Arg::Null => q.bind(Option::<String>::None),
            value::Arg::Int(i) => q.bind(*i),
            value::Arg::Float(f) => q.bind(*f),
            value::Arg::Bool(b) => q.bind(*b),
            value::Arg::Text(t) => q.bind(t.as_str()),
        };
    }
    q
}

/// `p1, p2, ...` — one per positional argument, in the platform's own
/// placeholder syntax.
fn placeholder_list(p: &dyn Platform, n: usize) -> String {
    (1..=n)
        .map(|i| p.placeholder(i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Calls a procedure: `CALL name(...)` with positional arguments.
pub async fn call_procedure(w: &mut World, name: &str, args_str: &str) -> Result<(), String> {
    let args = value::parse_args(args_str)?;
    let db = w.db.resources()?;
    let state = db.connection(w.db.current())?;
    let sql = format!(
        "CALL {name}({})",
        placeholder_list(state.platform, args.len())
    );
    if w.debug {
        eprintln!("SQL: {sql}\nARGUMENTS: {args:?}");
    }
    bind_args(sqlx::query(&sql), &args)
        .execute(&state.pool)
        .await
        .map_err(|e| format!("CALL {name}: {e}"))?;
    Ok(())
}

/// Calls a function, reading its result (cast to text via `Platform::cast_text`)
/// into a variable.
pub async fn call_function(
    w: &mut World,
    name: &str,
    args_str: &str,
    var: &str,
) -> Result<(), String> {
    let args = value::parse_args(args_str)?;
    let db = w.db.resources()?;
    let state = db.connection(w.db.current())?;
    let call = format!("{name}({})", placeholder_list(state.platform, args.len()));
    let sql = state.platform.cast_text(&call);
    let sql = format!("SELECT {sql}");
    if w.debug {
        eprintln!("SQL: {sql}\nARGUMENTS: {args:?}");
    }
    let row = bind_args(sqlx::query(&sql), &args)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| format!("SELECT {name}(...): {e}"))?;
    let v: Option<String> = row
        .try_get(0)
        .map_err(|e| format!("reading function result: {e}"))?;
    let value = v.unwrap_or_default();
    w.vars.set(var, value);
    Ok(())
}

/// Advances a sequence via `Platform::next_sequence`; stores the value in a variable.
pub async fn next_sequence(w: &mut World, seq: &str, var: &str) -> Result<(), String> {
    let db = w.db.resources()?;
    let state = db.connection(w.db.current())?;
    let (sql, binds) = state
        .platform
        .next_sequence(seq)
        .ok_or_else(|| format!("sequences are not supported on {}", state.platform.name()))?;
    if w.debug {
        eprintln!("SQL: {sql} [{seq}]");
    }
    let row = bind_all(sqlx::query(&sql), &binds)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| format!("nextval({seq}): {e}"))?;
    let v: String = row
        .try_get(0)
        .map_err(|e| format!("reading sequence: {e}"))?;
    w.vars.set(var, v);
    Ok(())
}
