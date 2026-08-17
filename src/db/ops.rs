use crate::db::plan::{self, InsertPlan};
use crate::db::reference::TableRef;
use crate::db::value;
use crate::world::World;
use sqlx::{PgPool, Postgres, Row, postgres::PgArguments, query::Query};
use std::sync::Arc;

/// Binds text parameters (`Option<String>`). The type cast `$N::type` happens
/// in the SQL itself, so everything is bound as text here.
pub fn bind_all<'q>(
    mut q: Query<'q, Postgres, PgArguments>,
    binds: &'q [Option<String>],
) -> Query<'q, Postgres, PgArguments> {
    for b in binds {
        q = q.bind(b);
    }
    q
}

/// Prints the SQL and parameters if debug mode is on (§8).
pub fn log_sql(w: &World, sql: &str, binds: &[Option<String>], logs: &[String]) {
    if w.debug {
        eprintln!("SQL: {sql}");
        eprintln!("PARAMS: {binds:?}");
        for l in logs {
            eprintln!("  auto: {l}");
        }
    }
}

/// Resolves a reference into (pool, schema, parsed reference). The connection comes
/// from the reference's prefix, or from the scenario's current connection.
pub async fn resolve<'a>(
    w: &'a World,
    raw_table: &str,
) -> Result<(&'a PgPool, Arc<plan::TableSchema>, TableRef), String> {
    let tref = TableRef::parse(raw_table)?;
    let conn = tref.conn.clone().unwrap_or_else(|| w.db.current().to_string());
    let suite = w.db.suite()?;
    let pool = suite.pool(&conn)?;
    let schema = suite.schema(&conn, &tref.sql_name()).await?;
    Ok((pool, schema, tref))
}

/// Inserts one row; stores the PK values in `last_insert_*`.
pub async fn insert(
    w: &mut World,
    raw_table: &str,
    values: &[(String, Option<String>)],
    index: Option<usize>,
) -> Result<(), String> {
    let (pool, schema, tref) = resolve(w, raw_table).await?;
    let InsertPlan { sql, binds, var_names, logs } =
        plan::build_insert(&schema, &tref.sql_name(), &tref.table, values, index)?;
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
    // PK values come back as text (RETURNING (col)::text) in var_names order.
    let mut assignments: Vec<(String, String)> = Vec::new();
    for (i, name) in var_names.iter().enumerate() {
        let v: String = row.try_get(i).map_err(|e| format!("reading RETURNING: {e}"))?;
        assignments.push((name.clone(), v));
    }
    // The pool borrow has ended — now it's safe to write to the variables.
    for (name, v) in assignments {
        w.vars.set(&name, v);
    }
    Ok(())
}

/// UPDATE ... SET ... WHERE ...; stores the affected row count in `updated_<table>`.
pub async fn update(
    w: &mut World,
    raw_table: &str,
    set: &str,
    where_: &str,
) -> Result<(), String> {
    let (pool, schema, tref) = resolve(w, raw_table).await?;
    let set_pairs = value::parse_oneliner(set)?;
    let where_pairs = value::parse_oneliner(where_)?;
    let (sql, binds) = plan::build_update(&schema, &tref.sql_name(), &set_pairs, &where_pairs)?;
    log_sql(w, &sql, &binds, &[]);
    let done = bind_all(sqlx::query(&sql), &binds)
        .execute(pool)
        .await
        .map_err(|e| format!("UPDATE {}: {e}", tref.sql_name()))?;
    let table = tref.table.clone();
    w.vars.set(&format!("updated_{table}"), done.rows_affected().to_string());
    Ok(())
}

/// DELETE FROM ... WHERE ... (an empty WHERE is forbidden by the builder);
/// stores the deleted row count in `deleted_<table>`.
pub async fn delete(w: &mut World, raw_table: &str, where_: &str) -> Result<(), String> {
    let (pool, schema, tref) = resolve(w, raw_table).await?;
    let where_pairs = value::parse_oneliner(where_)?;
    let (sql, binds) = plan::build_delete(&schema, &tref.sql_name(), &where_pairs)?;
    log_sql(w, &sql, &binds, &[]);
    let done = bind_all(sqlx::query(&sql), &binds)
        .execute(pool)
        .await
        .map_err(|e| format!("DELETE {}: {e}", tref.sql_name()))?;
    let table = tref.table.clone();
    w.vars.set(&format!("deleted_{table}"), done.rows_affected().to_string());
    Ok(())
}

/// Checks whether a row exists or not. `negate=false` requires the row to
/// exist; `negate=true` requires it to be absent.
pub async fn assert_exists(
    w: &mut World,
    raw_table: &str,
    where_pairs: &[(String, Option<String>)],
    negate: bool,
) -> Result<(), String> {
    let (pool, schema, tref) = resolve(w, raw_table).await?;
    let (sql, binds) = plan::build_exists(&schema, &tref.sql_name(), where_pairs)?;
    log_sql(w, &sql, &binds, &[]);
    let found = bind_all(sqlx::query(&sql), &binds)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("existence check in {}: {e}", tref.sql_name()))?
        .is_some();
    match (found, negate) {
        (true, false) | (false, true) => Ok(()),
        (false, false) => Err(format!("expected a row in {}, but none was found", tref.sql_name())),
        (true, true) => Err(format!("a row exists in {} but should not", tref.sql_name())),
    }
}

/// SELECT (column)::text FROM ... WHERE ... LIMIT 1; stores the value in a variable.
pub async fn extract(
    w: &mut World,
    column: &str,
    raw_table: &str,
    where_str: &str,
    var: &str,
) -> Result<(), String> {
    let (pool, schema, tref) = resolve(w, raw_table).await?;
    if schema.col(column).is_none() {
        return Err(format!("column {column:?} does not exist in {}", tref.sql_name()));
    }
    let where_pairs = value::parse_oneliner(where_str)?;
    let (where_sql, binds) = plan::build_where(&schema, &where_pairs, 1)?;
    let sql = format!("SELECT ({column})::text FROM {} WHERE {where_sql} LIMIT 1", tref.sql_name());
    log_sql(w, &sql, &binds, &[]);
    let row = bind_all(sqlx::query(&sql), &binds)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("SELECT {}: {e}", tref.sql_name()))?
        .ok_or_else(|| format!("no row matching the condition was found in {}", tref.sql_name()))?;
    let v: Option<String> = row.try_get(0).map_err(|e| format!("reading the value: {e}"))?;
    let value = v.unwrap_or_default();
    w.vars.set(var, value);
    Ok(())
}

/// DELETE FROM ... with no WHERE — clears the table entirely;
/// stores the deleted row count in `deleted_<table>`.
pub async fn delete_all(w: &mut World, raw_table: &str) -> Result<(), String> {
    let (pool, _schema, tref) = resolve(w, raw_table).await?;
    let sql = plan::build_delete_all(&tref.sql_name());
    log_sql(w, &sql, &[], &[]);
    let done = sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| format!("DELETE ALL {}: {e}", tref.sql_name()))?;
    let table = tref.table.clone();
    w.vars.set(&format!("deleted_{table}"), done.rows_affected().to_string());
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

/// `$1, $2, ...` — placeholders for `n` positional arguments.
fn placeholders(n: usize) -> String {
    (1..=n).map(|i| format!("${i}")).collect::<Vec<_>>().join(", ")
}

/// Calls a procedure: `CALL name(...)` with positional arguments.
pub async fn call_procedure(w: &mut World, name: &str, args_str: &str) -> Result<(), String> {
    let args = value::parse_args(args_str)?;
    let sql = format!("CALL {name}({})", placeholders(args.len()));
    if w.debug {
        eprintln!("SQL: {sql}\nARGS: {args:?}");
    }
    let suite = w.db.suite()?;
    let pool = suite.pool(w.db.current())?;
    bind_args(sqlx::query(&sql), &args)
        .execute(pool)
        .await
        .map_err(|e| format!("CALL {name}: {e}"))?;
    Ok(())
}

/// Calls a function `SELECT (name(...))::text` and stores the result in a variable.
pub async fn call_function(w: &mut World, name: &str, args_str: &str, var: &str) -> Result<(), String> {
    let args = value::parse_args(args_str)?;
    let sql = format!("SELECT ({name}({}))::text", placeholders(args.len()));
    if w.debug {
        eprintln!("SQL: {sql}\nARGS: {args:?}");
    }
    let suite = w.db.suite()?;
    let pool = suite.pool(w.db.current())?;
    let row = bind_args(sqlx::query(&sql), &args)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("SELECT {name}(...): {e}"))?;
    let v: Option<String> = row.try_get(0).map_err(|e| format!("reading the function result: {e}"))?;
    let value = v.unwrap_or_default();
    w.vars.set(var, value);
    Ok(())
}

/// Takes `nextval(seq)` and stores the value in a variable.
pub async fn next_sequence(w: &mut World, seq: &str, var: &str) -> Result<(), String> {
    let sql = "SELECT nextval($1::regclass)::text";
    if w.debug {
        eprintln!("SQL: {sql} [{seq}]");
    }
    let suite = w.db.suite()?;
    let pool = suite.pool(w.db.current())?;
    let row = sqlx::query(sql)
        .bind(seq)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("nextval({seq}): {e}"))?;
    let v: String = row.try_get(0).map_err(|e| format!("reading the sequence: {e}"))?;
    w.vars.set(var, v);
    Ok(())
}
