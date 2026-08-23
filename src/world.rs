use crate::db::DbHandle;
use crate::http::{Apis, HttpState};
use crate::unique::Generator;
use crate::vars::VarStack;
use std::sync::Arc;

/// Variables live for the feature file; HTTP state, the current DB connection,
/// and debug mode live for the scenario. Asymmetry from Behat's behavior (spec §3).
pub struct World {
    pub vars: VarStack,
    pub http: HttpState,
    pub db: DbHandle,
    pub debug: bool,
    pub generator: Arc<Generator>,
    /// Run-wide configuration, not scenario state: `reset_scenario`
    /// does not touch it.
    pub srp: Option<Arc<crate::srp::SrpParams>>,
}

impl World {
    pub fn new(
        apis: Arc<Apis>,
        generator: Arc<Generator>,
        db: DbHandle,
        srp: Option<Arc<crate::srp::SrpParams>>,
    ) -> Self {
        Self {
            vars: VarStack::new(),
            http: HttpState::new(apis),
            db,
            debug: false,
            generator,
            srp,
        }
    }

    /// At the scenario boundary: the request, connection, and debug flag reset; variables remain.
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
        Arc::new(Apis::new(by_name, Some("main".to_string())).expect("default declared"))
    }

    #[test]
    fn reset_scenario_clears_debug() {
        let mut w = World::new(
            apis(),
            Arc::new(Generator::new()),
            DbHandle::new(None, String::new()),
            None,
        );
        w.debug = true;
        w.reset_scenario();
        assert!(!w.debug, "debug resets at the scenario boundary");
    }

    #[test]
    fn srp_parameters_survive_a_scenario_boundary() {
        let params = Arc::new(crate::srp::SrpParams {
            variant: crate::srp::Variant::HexString,
            prime: num_bigint::BigUint::from(23u32),
            generator: num_bigint::BigUint::from(5u32),
            hash: crate::srp::HashAlg::Sha256,
        });
        let mut w = World::new(
            apis(),
            Arc::new(Generator::new()),
            DbHandle::new(None, String::new()),
            Some(params),
        );
        w.reset_scenario();
        assert!(
            w.srp.is_some(),
            "SRP parameters are run-wide configuration, not scenario state"
        );
    }
}
