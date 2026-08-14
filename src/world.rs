use crate::http::HttpState;
use crate::unique::Generator;
use crate::vars::VarStack;
use anyhow::Result;
use std::sync::Arc;

/// Variables live for the feature file; HTTP state lives for the scenario.
/// The asymmetry is taken from Behat's behavior, confirmed against the Imbo sources.
pub struct World {
    pub vars: VarStack,
    pub http: HttpState,
    pub generator: Arc<Generator>,
}

impl World {
    pub fn new(base_url: &str, timeout_secs: u64, generator: Arc<Generator>) -> Result<Self> {
        Ok(Self {
            vars: VarStack::new(),
            http: HttpState::new(base_url, timeout_secs)?,
            generator,
        })
    }

    /// Called at the scenario boundary: the request resets, variables remain.
    pub fn reset_scenario(&mut self, base_url: &str, timeout_secs: u64) -> Result<()> {
        self.http = HttpState::new(base_url, timeout_secs)?;
        Ok(())
    }
}
