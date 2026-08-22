use crate::db::DbHandle;
use crate::http::{Apis, HttpState};
use crate::unique::Generator;
use crate::vars::VarStack;
use std::sync::Arc;

/// Variables live for the feature file; HTTP state, the current DB connection, and
/// debug mode live for the scenario. The asymmetry comes from Behat's behavior (spec §3).
pub struct World {
    pub vars: VarStack,
    pub http: HttpState,
    pub db: DbHandle,
    pub debug: bool,
    pub generator: Arc<Generator>,
}

impl World {
    pub fn new(apis: Arc<Apis>, generator: Arc<Generator>, db: DbHandle) -> Self {
        Self {
            vars: VarStack::new(),
            http: HttpState::new(apis),
            db,
            debug: false,
            generator,
        }
    }

    /// At the scenario boundary: request, connection, and debug reset; variables remain.
    pub fn reset_scenario(&mut self) {
        self.http.reset();
        self.db.reset();
        self.debug = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;
    use crate::http::ApiResource;
    use crate::unique::Generator;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn apis() -> Arc<Apis> {
        let mut by_name = HashMap::new();
        by_name.insert(
            "main".to_string(),
            ApiResource::new("http://x.local", 5, Vec::new()).expect("valid base_url"),
        );
        Arc::new(Apis::new(by_name, Some("main".to_string())).expect("default is declared"))
    }

    #[test]
    fn reset_scenario_clears_debug() {
        let mut w = World::new(
            apis(),
            Arc::new(Generator::new()),
            DbHandle::new(None, String::new()),
        );
        w.debug = true;
        w.reset_scenario();
        assert!(!w.debug, "debug resets at the scenario boundary");
    }
}
