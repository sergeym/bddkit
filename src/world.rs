use crate::db::DbHandle;
use crate::http::HttpState;
use crate::unique::Generator;
use crate::vars::VarStack;
use anyhow::Result;
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
    pub fn new(
        base_url: &str,
        timeout_secs: u64,
        generator: Arc<Generator>,
        db: DbHandle,
    ) -> Result<Self> {
        Ok(Self {
            vars: VarStack::new(),
            http: HttpState::new(base_url, timeout_secs)?,
            db,
            debug: false,
            generator,
        })
    }

    /// At the scenario boundary: the request, connection, and debug flag reset; variables remain.
    pub fn reset_scenario(&mut self, base_url: &str, timeout_secs: u64) -> Result<()> {
        self.http = HttpState::new(base_url, timeout_secs)?;
        self.db.reset();
        self.debug = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;
    use crate::unique::Generator;
    use std::sync::Arc;

    #[test]
    fn reset_scenario_clears_debug_and_connection() {
        let g = Arc::new(Generator::new());
        let mut w = World::new("http://x.local", 5, g, DbHandle::new(None)).unwrap();
        w.debug = true;
        w.reset_scenario("http://x.local", 5).unwrap();
        assert!(!w.debug, "debug resets at the scenario boundary");
    }
}
