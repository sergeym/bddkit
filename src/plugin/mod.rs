// The plugin layer is built bottom-up over several commits: the ABI types, the
// loader, and the registry all land before `main` wires any of it in. Scoped to
// this module so it cannot mask dead code anywhere else, and removed once
// `main` loads plugins — dispatching them is not enough, several items here
// (the lock reader, the artifact root, the ABI version) have no caller until
// then.
#![allow(dead_code)]

pub mod abi;
pub mod library;
pub mod lock;

use crate::config::InstanceSpec;
use crate::options::Options;
use abi::{DispatchResult, InitRequest, OptionsJson};
use anyhow::{Context, Result, bail};
use library::Library;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Group names the host serves itself. A plugin claiming one of these would
/// shadow `resources.api` or `resources.db`, so it is refused at startup.
/// `connection` is the built-in step word for the `db` group (`I use "<conn>"
/// connection`) — a plugin claiming it would build a group-switch step that
/// exactly overlaps the built-in one, so it is refused for the same reason.
const HOST_GROUPS: &[&str] = &["api", "db", "srp", "connection"];

/// Everything about plugins that lives for the whole run. Shared through
/// `RunContext`, so every field is `Send + Sync`: handles are `u64`, and the
/// only mutable state sits behind a `Mutex`.
#[derive(Debug)]
pub struct Plugins {
    libs: Vec<Library>,
    /// group -> index into `libs`
    groups: BTreeMap<String, usize>,
    /// (group, instance) -> declared config and resolved options
    declared: BTreeMap<(String, String), InstanceSpec>,
    /// group -> default instance name
    defaults: BTreeMap<String, String>,
    /// (group, instance) -> live handle. Empty until a scenario reaches for it.
    live: Mutex<BTreeMap<(String, String), u64>>,
    artifacts_root: PathBuf,
    artifacts_counter: AtomicUsize,
}

/// Shared across worker tasks behind an `Arc`, so losing either half has to
/// fail here rather than as a confusing trait error in the runner.
const _: fn() = || {
    fn both<T: Send + Sync>() {}
    both::<Plugins>();
};

impl Plugins {
    /// `groups_in_config` is every `resources.<group>` key the host does not
    /// serve itself. A group nothing claims is a startup error; a loaded plugin
    /// whose group is absent from the config is not — it is simply never used.
    pub fn load(
        entries: Vec<lock::LockEntry>,
        instances: &[InstanceSpec],
        groups_in_config: &[String],
    ) -> Result<Self> {
        let mut libs: Vec<Library> = Vec::with_capacity(entries.len());
        let mut groups: BTreeMap<String, usize> = BTreeMap::new();
        for entry in entries {
            let lib = Library::load(&entry.name, &entry.path)?;
            let index = libs.len();
            for group in &lib.manifest.groups {
                if HOST_GROUPS.contains(&group.as_str()) {
                    bail!(
                        "plugin {:?} claims the group {group:?}, which bddkit serves itself",
                        lib.name
                    );
                }
                if let Some(other) = groups.insert(group.clone(), index) {
                    bail!(
                        "plugins {:?} and {:?} both claim the resource group {group:?}",
                        libs[other].name,
                        lib.name
                    );
                }
            }
            libs.push(lib);
        }

        for group in groups_in_config {
            if !groups.contains_key(group) {
                bail!(
                    "the config declares resources.{group}, but no installed plugin serves the group {group:?}"
                );
            }
        }

        let mut declared = BTreeMap::new();
        for spec in instances {
            declared.insert((spec.group.clone(), spec.name.clone()), spec.clone());
        }

        let plugins = Self {
            libs,
            groups,
            declared,
            defaults: BTreeMap::new(),
            live: Mutex::new(BTreeMap::new()),
            artifacts_root: std::env::temp_dir(),
            artifacts_counter: AtomicUsize::new(0),
        };

        // Eager, cheap, no connections opened: a typo in the config must exit 2
        // before the first request, which lazy init alone would not give.
        //
        // The group is looked up, never indexed: `groups_in_config` is the
        // caller's list of config keys, and an instance whose group is missing
        // from it must produce this error rather than an index panic.
        for spec in instances {
            let lib = plugins
                .groups
                .get(&spec.group)
                .map(|index| &plugins.libs[*index])
                .with_context(|| {
                    format!(
                        "resources.{}.{}: no installed plugin serves the group {:?}",
                        spec.group, spec.name, spec.group
                    )
                })?;
            let request = plugins.init_request(spec)?;
            lib.validate_config(&request)?
                .map_err(|error| anyhow::anyhow!("resources.{}.{}: {error}", spec.group, spec.name))?;
        }
        Ok(plugins)
    }

    /// Where per-dispatch artifact directories are rooted. Set once from the
    /// run id so two runs never collide.
    pub fn set_artifacts_root(&mut self, root: PathBuf) {
        self.artifacts_root = root;
    }

    pub fn set_defaults(&mut self, defaults: BTreeMap<String, String>) {
        self.defaults = defaults;
    }

    pub fn defaults(&self) -> &BTreeMap<String, String> {
        &self.defaults
    }

    pub fn is_empty(&self) -> bool {
        self.libs.is_empty()
    }

    pub fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    /// `(lib index, step index, pattern, is assertion)` for registry loading.
    pub fn steps(&self) -> Vec<(usize, usize, String, bool)> {
        let mut out = Vec::new();
        for (lib_index, lib) in self.libs.iter().enumerate() {
            for (step_index, step) in lib.steps.iter().enumerate() {
                out.push((
                    lib_index,
                    step_index,
                    step.pattern.clone(),
                    step.is_assertion(),
                ));
            }
        }
        out
    }

    pub fn step_count(&self) -> usize {
        self.libs.iter().map(|l| l.steps.len()).sum()
    }

    pub fn group_of_step(&self, lib: usize, step: usize) -> &str {
        &self.libs[lib].steps[step].group
    }

    pub fn is_declared(&self, group: &str, instance: &str) -> bool {
        self.declared
            .contains_key(&(group.to_string(), instance.to_string()))
    }

    pub fn options_for(&self, group: &str, instance: &str) -> Result<&Options, String> {
        self.declared
            .get(&(group.to_string(), instance.to_string()))
            .map(|spec| &spec.options)
            .ok_or_else(|| undeclared(group, instance))
    }

    /// A fresh directory path per dispatch, from a process-global counter:
    /// two workers handed the same path would overwrite each other's evidence.
    /// The host does not create it — a plugin that writes calls `create_dir_all`
    /// first, and most steps never write anything.
    pub fn next_artifacts_dir(&self) -> String {
        let index = self.artifacts_counter.fetch_add(1, Ordering::Relaxed);
        self.artifacts_root
            .join(format!("{index:06}"))
            .display()
            .to_string()
    }

    /// The one blocking entry point. Everything FFI happens here, so the caller
    /// wraps exactly this in `spawn_blocking`.
    pub fn call_step(
        &self,
        group: &str,
        instance: &str,
        lib: usize,
        step: usize,
        request: &str,
    ) -> Result<DispatchResult, String> {
        let handle = self.handle(group, instance)?;
        self.libs[lib]
            .dispatch(handle, step as u32, request)
            .map_err(|error| format!("{error:#}"))
    }

    /// Lazily creates the instance. The `Mutex` is never held across the
    /// `init_instance` call: two workers may both build one, and the loser
    /// drops its own instance rather than serialising every worker behind a
    /// lock held across a network round trip.
    fn handle(&self, group: &str, instance: &str) -> Result<u64, String> {
        let key = (group.to_string(), instance.to_string());
        {
            let live = self.live.lock().expect("plugin instances");
            if let Some(handle) = live.get(&key) {
                return Ok(*handle);
            }
        }
        let spec = self
            .declared
            .get(&key)
            .ok_or_else(|| undeclared(group, instance))?;
        let lib_index = *self
            .groups
            .get(group)
            .ok_or_else(|| format!("no plugin serves the resource group {group:?}"))?;
        let request = self.init_request(spec).map_err(|error| format!("{error:#}"))?;
        let created = self.libs[lib_index]
            .init_instance(&request)
            .map_err(|error| format!("{error:#}"))?
            .map_err(|error| format!("resources.{group}.{instance}: {error}"))?;

        let mut live = self.live.lock().expect("plugin instances");
        // Another worker got there first: drop the loser's own instance, off
        // the lock, and answer with the winner's handle either way.
        if let Some(winner) = live.get(&key).copied() {
            drop(live);
            if let Err(error) = self.libs[lib_index].drop_instance(created) {
                eprintln!("warning: dropping a duplicate {group}.{instance} failed: {error:#}");
            }
            return Ok(winner);
        }
        live.insert(key, created);
        Ok(created)
    }

    /// Called at the scenario boundary. A plugin without `reset_scenario`
    /// answers Ok, and an instance nobody has created yet is not touched.
    pub fn reset_scenario(&self) {
        let live: Vec<((String, String), u64)> = {
            let live = self.live.lock().expect("plugin instances");
            live.iter().map(|(k, v)| (k.clone(), *v)).collect()
        };
        for ((group, _), handle) in live {
            if let Some(index) = self.groups.get(&group) {
                let _ = self.libs[*index].reset_scenario(handle);
            }
        }
    }

    /// Every instance that was created is dropped, including after a failed or
    /// `--fail-fast` run: a plugin's external process must not outlive the run.
    ///
    /// Call it once, after every worker has finished. It takes `&self`, so the
    /// registry stays usable afterwards, but a `call_step` after this point
    /// creates an instance nothing will ever drop. The single call site — after
    /// the pool drains — is what prevents that; a flag would mean a check
    /// inside the lock region for a hazard that cannot arise.
    pub fn shutdown(&self) {
        let live: Vec<((String, String), u64)> = {
            let mut live = self.live.lock().expect("plugin instances");
            std::mem::take(&mut *live).into_iter().collect()
        };
        for ((group, instance), handle) in live {
            if let Some(index) = self.groups.get(&group) {
                match self.libs[*index].drop_instance(handle) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        eprintln!("warning: dropping {group}.{instance} failed: {error}")
                    }
                    Err(error) => {
                        eprintln!("warning: dropping {group}.{instance} failed: {error:#}")
                    }
                }
            }
        }
    }

    #[cfg(test)]
    fn live_count(&self) -> usize {
        self.live.lock().expect("plugin instances").len()
    }

    fn init_request(&self, spec: &InstanceSpec) -> Result<String> {
        serde_json::to_string(&InitRequest {
            group: &spec.group,
            instance: &spec.name,
            config: &spec.config,
            options: OptionsJson::from(&spec.options),
        })
        .with_context(|| format!("failed to encode resources.{}.{}", spec.group, spec.name))
    }
}

fn undeclared(group: &str, instance: &str) -> String {
    format!("instance {instance:?} is not declared in resources.{group}")
}

/// The scenario's current instance per group. Selection is per group on
/// purpose: switching the browser must not disturb the selected widget instance.
pub struct PluginState {
    plugins: Option<Arc<Plugins>>,
    defaults: BTreeMap<String, String>,
    current: BTreeMap<String, String>,
}

impl PluginState {
    pub fn new(plugins: Option<Arc<Plugins>>) -> Self {
        let defaults = plugins
            .as_ref()
            .map(|p| p.defaults().clone())
            .unwrap_or_default();
        Self {
            plugins,
            current: defaults.clone(),
            defaults,
        }
    }

    /// `PluginState::new` already takes its defaults from `Plugins::defaults()`;
    /// nothing in production sets them again. Kept for tests that need a
    /// `PluginState` without loading a real `Plugins`.
    #[cfg(test)]
    pub fn set_defaults(&mut self, defaults: BTreeMap<String, String>) {
        self.current = defaults.clone();
        self.defaults = defaults;
    }

    pub fn plugins(&self) -> Option<&Arc<Plugins>> {
        self.plugins.as_ref()
    }

    pub fn current(&self, group: &str) -> Result<&str, String> {
        self.current.get(group).map(String::as_str).ok_or_else(|| {
            format!(
                "no instance of the resource group {group:?} is selected, and no default_{group} is set"
            )
        })
    }

    pub fn use_instance(&mut self, group: &str, name: &str) -> Result<(), String> {
        let declared = self
            .plugins
            .as_ref()
            .is_some_and(|p| p.is_declared(group, name));
        if !declared {
            return Err(undeclared(group, name));
        }
        self.current.insert(group.to_string(), name.to_string());
        Ok(())
    }

    #[cfg(test)]
    pub fn use_instance_unchecked(&mut self, group: &str, name: &str) {
        self.current.insert(group.to_string(), name.to_string());
    }

    /// Per invariant 2 the selection returns to `default_<group>`.
    pub fn reset(&mut self) {
        self.current = self.defaults.clone();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::InstanceSpec;
    use crate::options::Options;

    /// Same build as `library`'s own fixture helper; a unit test cannot reach
    /// the integration test helpers, and six lines beat a shared crate.
    pub fn fixture() -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = root.join("target/fixture-plugin");
        let out = std::process::Command::new(env!("CARGO"))
            .args(["build", "--manifest-path"])
            .arg(root.join("tests/fixtures/echo-plugin/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .output()
            .expect("cargo runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        target.join("debug").join(format!(
            "{}echo_plugin{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ))
    }

    pub fn entry() -> crate::plugin::lock::LockEntry {
        serde_yaml_ng::from_str(&format!("name: echo\npath: {}\n", fixture().display()))
            .expect("entry parses")
    }

    pub fn instance(name: &str, prefix: Option<&str>) -> InstanceSpec {
        InstanceSpec {
            group: "echo".to_string(),
            name: name.to_string(),
            config: match prefix {
                Some(p) => serde_json::json!({ "prefix": p }),
                None => serde_json::json!({}),
            },
            options: Options::default(),
        }
    }

    #[test]
    fn loads_a_plugin_and_maps_its_group() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        assert_eq!(plugins.step_count(), 3);
        assert_eq!(plugins.group_of_step(0, 0), "echo");
    }

    #[test]
    fn a_config_group_with_no_plugin_is_a_startup_error() {
        let error = Plugins::load(Vec::new(), &[instance("a", Some("p-"))], &["echo".into()])
            .expect_err("nothing claims the group");
        assert!(format!("{error:#}").contains("echo"), "{error:#}");
    }

    #[test]
    fn an_instance_in_an_unserved_group_is_an_error_not_a_panic() {
        // groups_in_config is the caller's list of config keys; the eager
        // validation loop must not index its way into a panic when an instance
        // names a group that list left out.
        let error = Plugins::load(Vec::new(), &[instance("a", Some("p-"))], &[])
            .expect_err("nothing serves the group");
        assert!(format!("{error:#}").contains("echo"), "{error:#}");
    }

    #[test]
    fn a_declared_instance_is_validated_eagerly() {
        // A typo must exit before the first request, without opening anything:
        // that is what validate_config buys over lazy init alone.
        let error = Plugins::load(vec![entry()], &[instance("a", None)], &["echo".into()])
            .expect_err("prefix missing");
        let text = format!("{error:#}");
        assert!(text.contains("prefix"), "{text}");
        assert!(text.contains("resources.echo.a"), "{text}");
    }

    #[test]
    fn an_instance_is_created_only_on_first_use() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        assert_eq!(plugins.live_count(), 0, "loading must not create instances");
        let result = plugins
            .call_step("echo", "a", 0, 0, r#"{"args":["x","name"],"debug":false}"#)
            .expect("dispatch");
        assert_eq!(result.status, abi::Status::Passed);
        assert_eq!(plugins.live_count(), 1);
    }

    #[test]
    fn a_second_call_reuses_the_same_instance() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        let request = r#"{"args":["3"],"debug":false}"#;
        let first = plugins
            .call_step("echo", "a", 0, 1, request)
            .expect("dispatch");
        let second = plugins
            .call_step("echo", "a", 0, 1, request)
            .expect("dispatch");
        assert_eq!(first.status, abi::Status::NotYet);
        assert_eq!(second.status, abi::Status::NotYet);
        assert!(
            second.error.unwrap_or_default().contains("2 of 3"),
            "the counter must survive between dispatches, i.e. one instance"
        );
        assert_eq!(plugins.live_count(), 1);
    }

    #[test]
    fn an_undeclared_instance_is_an_error_naming_the_group() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        let error = plugins
            .call_step("echo", "ghost", 0, 0, r#"{"args":[],"debug":false}"#)
            .expect_err("not declared");
        assert!(error.contains("ghost") && error.contains("echo"), "{error}");
    }

    #[test]
    fn shutdown_drops_every_live_instance() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        plugins
            .call_step("echo", "a", 0, 0, r#"{"args":["x","n"],"debug":false}"#)
            .expect("dispatch");
        plugins.shutdown();
        assert_eq!(plugins.live_count(), 0);
    }

    #[test]
    fn reset_scenario_reaches_live_instances_only() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        // Nothing initialised yet: this must be a no-op, not a failure.
        plugins.reset_scenario();
        let request = r#"{"args":["2"],"debug":false}"#;
        plugins
            .call_step("echo", "a", 0, 1, request)
            .expect("dispatch");
        plugins.reset_scenario();
        let after = plugins
            .call_step("echo", "a", 0, 1, request)
            .expect("dispatch");
        assert_eq!(
            after.status,
            abi::Status::NotYet,
            "the counter restarted, so attempt 1 of 2 is not there yet"
        );
    }

    #[test]
    fn each_dispatch_gets_its_own_artifacts_dir() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        let first = plugins.next_artifacts_dir();
        let second = plugins.next_artifacts_dir();
        assert_ne!(first, second, "two workers must never share an artifact path");
    }

    #[test]
    fn switching_to_a_declared_instance_selects_it_and_an_undeclared_one_is_an_error() {
        let plugins = Plugins::load(vec![entry()], &[instance("a", Some("p-"))], &["echo".into()])
            .expect("loads");
        let mut state = PluginState::new(Some(Arc::new(plugins)));
        state.use_instance("echo", "a").expect("declared instance");
        assert_eq!(state.current("echo").expect("selected"), "a");
        let error = state.use_instance("echo", "ghost").expect_err("undeclared");
        assert!(error.contains("ghost") && error.contains("echo"), "{error}");
        assert_eq!(
            state.current("echo").expect("selected"),
            "a",
            "a failed switch must not change the selection"
        );
    }
}
