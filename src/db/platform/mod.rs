mod postgres;

pub use postgres::PG;

use crate::db::plan::Column;
use crate::db::reference::TableRef;

/// The engine-specific SQL dialect. Every connection is reached through a
/// `&'static dyn Platform` — nothing is allocated per connection.
pub trait Platform: Send + Sync {
    fn name(&self) -> &'static str;
    fn bind(&self, n: usize, ty: &str) -> String;
    fn placeholder(&self, n: usize) -> String;
    fn cast_text(&self, expr: &str) -> String;
    fn insert_no_columns(&self, table: &str) -> String;
    fn returning(&self, pk: &[&Column]) -> Option<String>;
    fn is_timestamplike(&self, ty: &str) -> bool;
    fn wants_client_uuid(&self, col: &Column) -> bool;
    #[allow(dead_code)] // unused until MySQL, which refuses binary columns
    fn check_bindable(&self, col: &Column) -> Result<(), String>;

    /// The introspection query for one table, plus its binds. `db/introspect.rs`
    /// reads the result set by column name, with no compile-time check that the
    /// query actually produces them — a wrong alias here is a runtime panic
    /// there. The query MUST return exactly these six aliases:
    /// - `name` (text) — column name
    /// - `type_name` (text) — the platform's native type name, as used by `bind`/`returning`
    /// - `not_null` (bool)
    /// - `has_default` (bool)
    /// - `is_identity` (bool)
    /// - `is_pk` (bool)
    fn introspect(&self, tref: &TableRef) -> (String, Vec<Option<String>>);

    fn next_sequence(&self, seq: &str) -> Option<(String, Vec<Option<String>>)>;

    /// One or more session-setup statements to run right after connecting.
    /// `search_path` is `resources.db.<name>.search_path` from the config,
    /// verbatim. Empty input means no statement is needed.
    fn session_setup(&self, search_path: &[String]) -> Result<Vec<String>, String>;
}
