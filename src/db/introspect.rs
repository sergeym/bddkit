use crate::db::bind_all;
use crate::db::plan::{Column, TableSchema};
use crate::db::platform::Platform;
use crate::db::reference::TableRef;
use sqlx::{PgPool, Row};

pub async fn introspect(
    pool: &PgPool,
    platform: &dyn Platform,
    tref: &TableRef,
) -> Result<TableSchema, String> {
    let (sql, binds) = platform.introspect(tref);
    let rows = bind_all(sqlx::query(&sql), &binds)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("introspection of {}: {e}", tref.sql_name()))?;
    if rows.is_empty() {
        return Err(format!("table {} not found or inaccessible", tref.sql_name()));
    }
    let columns = rows
        .iter()
        .map(|r| Column {
            name: r.get("name"),
            type_name: r.get("type_name"),
            not_null: r.get("not_null"),
            has_default: r.get("has_default"),
            is_identity: r.get("is_identity"),
            is_pk: r.get("is_pk"),
        })
        .collect();
    Ok(TableSchema { columns })
}
