use crate::db::plan::{Column, TableSchema};
use sqlx::{PgPool, Row};

/// A single query against `pg_catalog`. `to_regclass($1)` resolves the name
/// honoring search_path; an empty result means "table not found".
const INTROSPECT_SQL: &str = "\
SELECT a.attname::text AS name,
       t.typname::text AS type_name,
       a.attnotnull AS not_null,
       (a.atthasdef OR a.attidentity <> '') AS has_default,
       (a.attidentity <> '') AS is_identity,
       EXISTS (
         SELECT 1 FROM pg_constraint c
         WHERE c.conrelid = a.attrelid AND c.contype = 'p' AND a.attnum = ANY (c.conkey)
       ) AS is_pk
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
WHERE a.attrelid = to_regclass($1) AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum";

pub async fn introspect(pool: &PgPool, sql_name: &str) -> Result<TableSchema, String> {
    let rows = sqlx::query(INTROSPECT_SQL)
        .bind(sql_name)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("introspection of {sql_name}: {e}"))?;
    if rows.is_empty() {
        return Err(format!("table {sql_name} not found or inaccessible"));
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
