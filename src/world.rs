use crate::db::DbHandle;
use crate::http::{Apis, HttpState};
use crate::options::{Options, OptionsLayer};
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
    pub options: Options,
    pending_options: Option<OptionsLayer>,
    /// Run-wide configuration, not scenario state: `reset_scenario`
    /// does not touch it.
    pub srp: Option<Arc<crate::srp::SrpParams>>,
    /// Scenario state: which instance of each plugin group is selected.
    pub plugins: crate::plugin::PluginState,
}

impl World {
    pub fn new(
        apis: Arc<Apis>,
        generator: Arc<Generator>,
        db: DbHandle,
        srp: Option<Arc<crate::srp::SrpParams>>,
        plugins: Option<Arc<crate::plugin::Plugins>>,
        options: Options,
    ) -> Self {
        Self {
            vars: VarStack::new(),
            http: HttpState::new(apis),
            db,
            debug: false,
            generator,
            srp,
            options,
            pending_options: None,
            plugins: crate::plugin::PluginState::new(plugins),
        }
    }

    pub fn arm_options(&mut self, layer: OptionsLayer) {
        self.pending_options = Some(layer);
    }

    pub fn take_options(&mut self) -> Option<OptionsLayer> {
        self.pending_options.take()
    }

    /// At the scenario boundary: the request, connection, and debug flag reset; variables remain.
    pub fn reset_scenario(&mut self) {
        self.http.reset();
        self.db.reset();
        self.debug = false;
        self.pending_options = None;
        self.plugins.reset();
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
            ApiResource::new("http://x.local", 5, Vec::new(), Options::default())
                .expect("valid base_url"),
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
            None,
            Options::default(),
        );
        w.debug = true;
        w.reset_scenario();
        assert!(!w.debug, "debug resets at the scenario boundary");
    }

    #[test]
    fn the_current_plugin_instance_resets_at_the_scenario_boundary() {
        // Invariant 2: a scenario never inherits the instance another one switched
        // to, exactly as the current API resource resets.
        let mut state = crate::plugin::PluginState::new(None);
        state.set_defaults([("echo".to_string(), "a".to_string())].into_iter().collect());
        state.use_instance_unchecked("echo", "b");
        assert_eq!(state.current("echo").expect("selected"), "b");
        state.reset();
        assert_eq!(state.current("echo").expect("selected"), "a");
    }

    #[test]
    fn reset_scenario_resets_the_plugin_selection() {
        let mut w = World::new(
            apis(),
            Arc::new(Generator::new()),
            DbHandle::new(None, String::new()),
            None,
            None,
            Options::default(),
        );
        w.plugins
            .set_defaults([("echo".to_string(), "a".to_string())].into_iter().collect());
        w.plugins.use_instance_unchecked("echo", "b");
        w.reset_scenario();
        assert_eq!(w.plugins.current("echo").expect("selected"), "a");
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
            None,
            Options::default(),
        );
        w.reset_scenario();
        assert!(
            w.srp.is_some(),
            "SRP parameters are run-wide configuration, not scenario state"
        );
    }
}
