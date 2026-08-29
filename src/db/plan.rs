use super::platform::Platform;

/// A table column from introspection data, as reported by the current
/// platform's `Platform::introspect` query. `type_name` is the platform's
/// native type name, as consumed by `Platform::bind`/`returning`.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub not_null: bool,
    pub has_default: bool,
    pub is_identity: bool,
    pub is_pk: bool,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub columns: Vec<Column>,
}

impl TableSchema {
    pub fn col(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
    pub fn pk_columns(&self) -> Vec<&Column> {
        self.columns.iter().filter(|c| c.is_pk).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertPlan {
    pub sql: String,
    pub binds: Vec<Option<String>>,
    pub var_names: Vec<String>,
    pub logs: Vec<String>,
}

/// Builds an INSERT: applies the PK and NOT NULL fill rules from spec §8.
/// `values` are the pairs given in the step (`None` = SQL NULL). `index` is the
/// variable-name suffix for the table form (`Some(0)` → `_0`).
pub fn build_insert(
    platform: &dyn Platform,
    schema: &TableSchema,
    sql_name: &str,
    bare: &str,
    values: &[(String, Option<String>)],
    index: Option<usize>,
) -> Result<InsertPlan, String> {
    // Given columns must exist.
    for (col, _) in values {
        if schema.col(col).is_none() {
            return Err(format!("column {col:?} is missing from table {sql_name}"));
        }
    }
    let given: std::collections::HashSet<&str> = values.iter().map(|(c, _)| c.as_str()).collect();

    let mut cols: Vec<String> = Vec::new();
    let mut exprs: Vec<String> = Vec::new();
    let mut binds: Vec<Option<String>> = Vec::new();
    let mut logs: Vec<String> = Vec::new();
    let mut param = 1usize;

    // 1. Given values — as-is, cast to the column's type.
    for (col, val) in values {
        let ty = &schema.col(col).expect("checked above").type_name;
        cols.push(col.clone());
        exprs.push(platform.bind(param, ty));
        binds.push(val.clone());
        param += 1;
    }

    // 2. Primary keys with no value.
    for pk in schema.pk_columns() {
        if given.contains(pk.name.as_str()) {
            continue;
        }
        if pk.has_default || pk.is_identity {
            continue; // the platform will generate it itself (default or identity/auto-increment).
        }
        if platform.wants_client_uuid(pk) {
            let id = uuid::Uuid::now_v7().to_string();
            logs.push(format!("PK {} := {id} (UUIDv7)", pk.name));
            cols.push(pk.name.clone());
            exprs.push(platform.bind(param, &pk.type_name));
            binds.push(Some(id));
            param += 1;
        } else {
            return Err(format!(
                "primary key {} ({}) has no value and is not generated (no default, not uuid)",
                pk.name, pk.type_name
            ));
        }
    }

    // 3. NOT NULL with no value and no default.
    for c in &schema.columns {
        if given.contains(c.name.as_str()) || c.is_pk || c.has_default || !c.not_null {
            continue;
        }
        if platform.is_timestamplike(&c.type_name) {
            logs.push(format!("{} := now()", c.name));
            cols.push(c.name.clone());
            // "now()" is portable across Postgres, MySQL and MariaDB — no need
            // to route it through Platform.
            exprs.push("now()".to_string());
        } else {
            return Err(format!(
                "column {} ({}) is NOT NULL, has no value and no default",
                c.name, c.type_name
            ));
        }
    }

    // 4. Assemble the SQL.
    let pk_cols = schema.pk_columns();
    let body = if cols.is_empty() {
        platform.insert_no_columns(sql_name)
    } else {
        format!(
            "INSERT INTO {sql_name} ({}) VALUES ({})",
            cols.join(", "),
            exprs.join(", ")
        )
    };
    let sql = match platform.returning(&pk_cols) {
        Some(clause) => format!("{body} {clause}"),
        None => body,
    };

    // 5. Variable names for RETURNING.
    let suffix = index.map(|i| format!("_{i}")).unwrap_or_default();
    let var_names: Vec<String> = if pk_cols.len() == 1 {
        vec![format!("last_insert_id_{bare}{suffix}")]
    } else {
        pk_cols
            .iter()
            .map(|c| format!("last_insert_{bare}_{}{suffix}", c.name))
            .collect()
    };

    Ok(InsertPlan {
        sql,
        binds,
        var_names,
        logs,
    })
}

/// Builds `col = <bind> AND …`, in the platform's own bind syntax. NULL →
/// `col IS NULL` with no bind. Parameter numbering starts at `start` (for
/// UPDATE, where SET takes the first parameter).
pub fn build_where(
    platform: &dyn Platform,
    schema: &TableSchema,
    pairs: &[(String, Option<String>)],
    start: usize,
) -> Result<(String, Vec<Option<String>>), String> {
    let mut parts = Vec::new();
    let mut binds = Vec::new();
    let mut param = start;
    for (col, val) in pairs {
        let c = schema
            .col(col)
            .ok_or_else(|| format!("column {col:?} is missing from the table"))?;
        match val {
            None => parts.push(format!("{col} IS NULL")),
            Some(_) => {
                parts.push(format!("{col} = {}", platform.bind(param, &c.type_name)));
                binds.push(val.clone());
                param += 1;
            }
        }
    }
    Ok((parts.join(" AND "), binds))
}

pub fn build_update(
    platform: &dyn Platform,
    schema: &TableSchema,
    sql_name: &str,
    set: &[(String, Option<String>)],
    where_: &[(String, Option<String>)],
) -> Result<(String, Vec<Option<String>>), String> {
    if where_.is_empty() {
        return Err("UPDATE without WHERE is forbidden; give a condition for a bulk change".into());
    }
    let mut sets = Vec::new();
    let mut binds = Vec::new();
    let mut param = 1usize;
    for (col, val) in set {
        let c = schema
            .col(col)
            .ok_or_else(|| format!("column {col:?} is missing from the table"))?;
        sets.push(format!("{col} = {}", platform.bind(param, &c.type_name)));
        binds.push(val.clone());
        param += 1;
    }
    let (where_sql, where_binds) = build_where(platform, schema, where_, param)?;
    binds.extend(where_binds);
    Ok((
        format!(
            "UPDATE {sql_name} SET {} WHERE {where_sql}",
            sets.join(", ")
        ),
        binds,
    ))
}

pub fn build_delete(
    platform: &dyn Platform,
    schema: &TableSchema,
    sql_name: &str,
    where_: &[(String, Option<String>)],
) -> Result<(String, Vec<Option<String>>), String> {
    if where_.is_empty() {
        return Err("DELETE without WHERE is forbidden; use the \"I delete all\" step for a full wipe".into());
    }
    let (where_sql, binds) = build_where(platform, schema, where_, 1)?;
    Ok((format!("DELETE FROM {sql_name} WHERE {where_sql}"), binds))
}

pub fn build_delete_all(sql_name: &str) -> String {
    format!("DELETE FROM {sql_name}")
}

pub fn build_exists(
    platform: &dyn Platform,
    schema: &TableSchema,
    sql_name: &str,
    where_: &[(String, Option<String>)],
) -> Result<(String, Vec<Option<String>>), String> {
    if where_.is_empty() {
        return Err("an existence check requires a condition".into());
    }
    let (where_sql, binds) = build_where(platform, schema, where_, 1)?;
    Ok((
        format!("SELECT 1 FROM {sql_name} WHERE {where_sql} LIMIT 1"),
        binds,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::platform::PG;
    use super::*;

    fn col(
        name: &str,
        ty: &str,
        not_null: bool,
        has_default: bool,
        is_identity: bool,
        is_pk: bool,
    ) -> Column {
        Column {
            name: name.into(),
            type_name: ty.into(),
            not_null,
            has_default,
            is_identity,
            is_pk,
        }
    }

    // users(id uuid pk with no default, email text not null, created_at timestamptz not null)
    fn users_uuid() -> TableSchema {
        TableSchema {
            columns: vec![
                col("id", "uuid", true, false, false, true),
                col("email", "text", true, false, false, false),
                col("created_at", "timestamptz", true, false, false, false),
            ],
        }
    }

    #[test]
    fn generates_uuid_pk_and_fills_timestamp() {
        let p = build_insert(
            &PG,
            &users_uuid(),
            "users",
            "users",
            &[("email".into(), Some("a@b.net".into()))],
            None,
        )
        .unwrap();
        // email is given ($1), id is generated ($2::uuid), created_at → now()
        assert!(
            p.sql.starts_with(
                "INSERT INTO users (email, id, created_at) VALUES ($1::text, $2::uuid, now())"
            ),
            "{}",
            p.sql
        );
        assert!(p.sql.ends_with("RETURNING (id)::text"), "{}", p.sql);
        assert_eq!(p.binds.len(), 2);
        assert_eq!(p.binds[0], Some("a@b.net".to_string()));
        assert!(uuid::Uuid::parse_str(p.binds[1].as_ref().unwrap()).is_ok());
        assert_eq!(p.var_names, vec!["last_insert_id_users"]);
    }

    #[test]
    fn omits_identity_and_default_pk() {
        // companies(id int identity pk, slug text not null)
        let s = TableSchema {
            columns: vec![
                col("id", "int4", true, true, true, true),
                col("slug", "text", true, false, false, false),
            ],
        };
        let p = build_insert(
            &PG,
            &s,
            "companies",
            "companies",
            &[("slug".into(), Some("x".into()))],
            None,
        )
        .unwrap();
        assert_eq!(
            p.sql,
            "INSERT INTO companies (slug) VALUES ($1::text) RETURNING (id)::text"
        );
        assert_eq!(p.var_names, vec!["last_insert_id_companies"]);
    }

    #[test]
    fn missing_plain_not_null_is_error() {
        let s = TableSchema {
            columns: vec![
                col("id", "int4", true, true, true, true),
                col("qty", "int4", true, false, false, false),
            ],
        };
        let err = build_insert(&PG, &s, "t", "t", &[], None).unwrap_err();
        assert!(err.contains("qty"), "{err}");
    }

    #[test]
    fn unknown_column_is_error() {
        let err = build_insert(
            &PG,
            &users_uuid(),
            "users",
            "users",
            &[("nope".into(), Some("1".into()))],
            None,
        )
        .unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn provided_null_binds_none() {
        let s = TableSchema {
            columns: vec![
                col("id", "int4", true, true, true, true),
                col("deleted_at", "timestamptz", false, false, false, false),
            ],
        };
        let p = build_insert(&PG, &s, "t", "t", &[("deleted_at".into(), None)], None).unwrap();
        assert_eq!(p.binds, vec![None]);
        assert!(p.sql.contains("$1::timestamptz"), "{}", p.sql);
    }

    #[test]
    fn composite_pk_yields_per_column_vars() {
        let s = TableSchema {
            columns: vec![
                col("a", "int4", true, false, false, true),
                col("b", "int4", true, false, false, true),
            ],
        };
        let p = build_insert(
            &PG,
            &s,
            "pair",
            "pair",
            &[
                ("a".into(), Some("1".into())),
                ("b".into(), Some("2".into())),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            p.var_names,
            vec!["last_insert_pair_a", "last_insert_pair_b"]
        );
        assert!(
            p.sql.ends_with("RETURNING (a)::text, (b)::text"),
            "{}",
            p.sql
        );
    }

    #[test]
    fn table_index_suffixes_var_name() {
        let p = build_insert(
            &PG,
            &users_uuid(),
            "users",
            "users",
            &[("email".into(), Some("a@b.net".into()))],
            Some(3),
        )
        .unwrap();
        assert_eq!(p.var_names, vec!["last_insert_id_users_3"]);
    }

    fn companies() -> TableSchema {
        TableSchema {
            columns: vec![
                col("id", "int4", true, true, true, true),
                col("slug", "text", true, false, false, false),
                col("deleted_at", "timestamptz", false, false, false, false),
            ],
        }
    }

    #[test]
    fn where_uses_typed_casts_and_is_null() {
        let (sql, binds) = build_where(
            &PG,
            &companies(),
            &[
                ("slug".into(), Some("x".into())),
                ("deleted_at".into(), None),
            ],
            1,
        )
        .unwrap();
        assert_eq!(sql, "slug = $1::text AND deleted_at IS NULL");
        assert_eq!(binds, vec![Some("x".to_string())]);
    }

    #[test]
    fn where_param_numbering_respects_start() {
        let (sql, _) = build_where(&PG, &companies(), &[("slug".into(), Some("x".into()))], 4).unwrap();
        assert_eq!(sql, "slug = $4::text");
    }

    #[test]
    fn where_unknown_column_is_error() {
        assert!(build_where(&PG, &companies(), &[("nope".into(), Some("1".into()))], 1).is_err());
    }

    #[test]
    fn update_sets_then_where_numbering() {
        let (sql, binds) = build_update(
            &PG,
            &companies(),
            "companies",
            &[("slug".into(), Some("new".into()))],
            &[("id".into(), Some("7".into()))],
        )
        .unwrap();
        assert_eq!(
            sql,
            "UPDATE companies SET slug = $1::text WHERE id = $2::int4"
        );
        assert_eq!(binds, vec![Some("new".to_string()), Some("7".to_string())]);
    }

    #[test]
    fn update_requires_where() {
        assert!(
            build_update(
                &PG,
                &companies(),
                "companies",
                &[("slug".into(), Some("x".into()))],
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn delete_requires_where() {
        assert!(build_delete(&PG, &companies(), "companies", &[]).is_err());
    }

    #[test]
    fn delete_all_has_no_where() {
        assert_eq!(build_delete_all("companies"), "DELETE FROM companies");
    }

    #[test]
    fn exists_selects_one() {
        let (sql, _) = build_exists(
            &PG,
            &companies(),
            "companies",
            &[("slug".into(), Some("x".into()))],
        )
        .unwrap();
        assert_eq!(sql, "SELECT 1 FROM companies WHERE slug = $1::text LIMIT 1");
    }
}
