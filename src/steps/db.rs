use crate::db::{ops, value};
use crate::world::World;

/// Switches the scenario's current connection. An error if the name is unknown.
pub fn use_connection(w: &mut World, name: &str) -> Result<(), String> {
    w.db.set_current(name)
}

pub fn debug_on(w: &mut World) -> Result<(), String> {
    w.debug = true;
    Ok(())
}

pub fn debug_off(w: &mut World) -> Result<(), String> {
    w.debug = false;
    Ok(())
}

pub async fn have_with(w: &mut World, table: &str, kv: &str) -> Result<(), String> {
    let values = value::parse_oneliner(kv)?;
    ops::insert(w, table, &values, None).await
}

pub async fn have_where(
    w: &mut World,
    table: &str,
    rows: Option<&Vec<Vec<String>>>,
) -> Result<(), String> {
    let rows = rows.ok_or("step requires a table")?;
    let sets = value::pairs_from_wide(rows)?;
    for (i, values) in sets.iter().enumerate() {
        ops::insert(w, table, values, Some(i)).await?;
    }
    Ok(())
}

pub async fn have_multi(w: &mut World, rows: Option<&Vec<Vec<String>>>) -> Result<(), String> {
    let rows = rows.ok_or("step requires a table")?;
    // The first row is a header (| table | record |), then (table, kv) pairs follow.
    for row in rows.iter().skip(1) {
        let table = row.first().ok_or("empty table row")?;
        let kv = row.get(1).map(String::as_str).unwrap_or("");
        let values = value::parse_oneliner(kv)?;
        ops::insert(w, table, &values, None).await?;
    }
    Ok(())
}

pub async fn update(w: &mut World, table: &str, set: &str, where_: &str) -> Result<(), String> {
    ops::update(w, table, set, where_).await
}

pub async fn delete_where(w: &mut World, table: &str, where_: &str) -> Result<(), String> {
    ops::delete(w, table, where_).await
}

pub async fn delete_all(w: &mut World, table: &str) -> Result<(), String> {
    ops::delete_all(w, table).await
}

pub async fn should_have(w: &mut World, table: &str, kv: &str) -> Result<(), String> {
    let pairs = value::parse_oneliner(kv)?;
    ops::assert_exists(w, table, &pairs, false).await
}

pub async fn should_not_have(w: &mut World, table: &str, kv: &str) -> Result<(), String> {
    let pairs = value::parse_oneliner(kv)?;
    ops::assert_exists(w, table, &pairs, true).await
}

pub async fn should_have_table(
    w: &mut World,
    table: &str,
    rows: Option<&Vec<Vec<String>>>,
) -> Result<(), String> {
    let pairs = value::pairs_from_tall(rows.ok_or("step requires a table")?)?;
    ops::assert_exists(w, table, &pairs, false).await
}

pub async fn should_not_have_table(
    w: &mut World,
    table: &str,
    rows: Option<&Vec<Vec<String>>>,
) -> Result<(), String> {
    let pairs = value::pairs_from_tall(rows.ok_or("step requires a table")?)?;
    ops::assert_exists(w, table, &pairs, true).await
}
