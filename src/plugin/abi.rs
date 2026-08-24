//! Everything that crosses the FFI boundary. Only JSON strings cross it:
//! `rustc` has no stable ABI, so the contract is a documented JSON schema
//! plus the C representation of a pointer, and nothing else.
//!
//! None of these types set `deny_unknown_fields`: a newer plugin may send
//! extra keys an older host doesn't know about yet, and those must be
//! ignored rather than fail the parse. This is the opposite choice from
//! `src/options.rs`, which deliberately rejects unknown keys to catch a typo
//! in hand-written YAML — that asymmetry is intentional, not an oversight to
//! "clean up".

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bumped only for a breaking change to the payloads or the symbol set.
/// The host refuses a plugin reporting anything else.
pub const ABI_VERSION: u32 = 1;

/// A message used whenever a plugin reports failure without giving a reason.
const NO_MESSAGE: &str = "the plugin reported a failure with no message";

/// The loader must refuse a manifest declaring `PerWorker` rather than
/// silently treating it as `Shared`: that would hand an instance the plugin
/// declared is *not* thread-safe to several concurrent workers at once — a
/// data race inside someone else's cdylib, surfacing as a flaky suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
    /// One instance serves the whole run; the plugin guarantees its handle is
    /// safe to call from several workers at once.
    #[default]
    Shared,
    /// One instance per worker. Declared in the ABI but not implemented in P1.
    ///
    /// The value set is closed on purpose: a scheduling mode the host does not
    /// implement cannot be degraded into one it does, so a new mode comes with
    /// an `ABI_VERSION` bump rather than with a tolerant parse.
    PerWorker,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub groups: Vec<String>,
    #[serde(default)]
    pub concurrency: Concurrency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Action,
    Assertion,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepSpec {
    pub pattern: String,
    pub group: String,
    pub kind: StepKind,
}

impl StepSpec {
    pub fn is_assertion(&self) -> bool {
        matches!(self.kind, StepKind::Assertion)
    }
}

/// The polling options the host resolved for this instance. Serialised for the
/// plugin's information only: the sleep loop itself always stays on the host.
#[derive(Debug, Clone, Serialize)]
pub struct PollingJson {
    pub timeout_secs: u64,
    pub interval_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptionsJson {
    pub polling: PollingJson,
}

impl From<&crate::options::Options> for OptionsJson {
    fn from(options: &crate::options::Options) -> Self {
        Self {
            polling: PollingJson {
                timeout_secs: options.polling.timeout.as_secs(),
                interval_ms: options.polling.interval.as_millis() as u64,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRequest<'a> {
    pub group: &'a str,
    pub instance: &'a str,
    pub config: &'a serde_json::Value,
    pub options: OptionsJson,
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatchRequest<'a> {
    pub args: &'a [String],
    pub docstring: Option<&'a String>,
    pub table: Option<&'a Vec<Vec<String>>>,
    pub artifacts_dir: String,
    pub debug: bool,
    pub options: OptionsJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    NotYet,
    Fatal,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Diagnostic {
    pub title: String,
    /// `text | json | http | image`. Not an enum: an unknown kind from a newer
    /// plugin must degrade to "print it as text", never fail the parse.
    pub kind: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DispatchResult {
    pub status: Status,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub error: Option<String>,
}

impl DispatchResult {
    /// Diagnostics are evidence on the failure path, so they have to end up
    /// inside the one string `report::render_file` writes. Printing them
    /// directly would land in the middle of another worker's dump.
    pub fn render_failure(&self) -> String {
        let mut out = self.error.clone().unwrap_or_else(|| NO_MESSAGE.to_string());
        for d in &self.diagnostics {
            out.push_str(&format!("\n\n--- {} ({}) ---", d.title, d.kind));
            if let Some(content) = &d.content {
                out.push('\n');
                out.push_str(content);
            }
            if let Some(path) = &d.path {
                out.push_str(&format!("\n{path}"));
            }
        }
        out
    }
}

/// The reply shape for everything that is not a step: configuration validation
/// is not an assertion and must not be able to answer `not_yet`.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

impl Envelope {
    pub fn into_result(self) -> Result<(), String> {
        if self.ok {
            Ok(())
        } else {
            Err(self.error.unwrap_or_else(|| NO_MESSAGE.to_string()))
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct InitResponse {
    pub ok: bool,
    #[serde(default)]
    pub handle: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

impl InitResponse {
    pub fn into_result(self) -> Result<u64, String> {
        if self.ok {
            self.handle
                .ok_or_else(|| "the plugin reported ok with no handle".to_string())
        } else {
            Err(self
                .error
                .unwrap_or_else(|| "the plugin refused to create the instance".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_parses() {
        let m: Manifest = serde_json::from_str(
            r#"{"name":"widget","version":"1.2.0","groups":["widget"],"concurrency":"shared"}"#,
        )
        .expect("manifest parses");
        assert_eq!(m.name, "widget");
        assert_eq!(m.groups, vec!["widget".to_string()]);
        assert_eq!(m.concurrency, Concurrency::Shared);
    }

    #[test]
    fn concurrency_defaults_to_shared() {
        // The field enters the ABI now so adding it later cannot break every
        // published plugin; a plugin that omits it means "shared".
        let m: Manifest =
            serde_json::from_str(r#"{"name":"x","version":"0.1.0","groups":["x"]}"#)
                .expect("manifest parses");
        assert_eq!(m.concurrency, Concurrency::Shared);
    }

    #[test]
    fn a_step_spec_parses_both_kinds() {
        let steps: Vec<StepSpec> = serde_json::from_str(
            r#"[{"pattern":"^a$","group":"widget","kind":"action"},
                {"pattern":"^b$","group":"widget","kind":"assertion"}]"#,
        )
        .expect("steps parse");
        assert!(!steps[0].is_assertion());
        assert!(steps[1].is_assertion());
        assert_eq!(steps[0].group, "widget");
    }

    #[test]
    fn a_dispatch_result_parses_with_defaults() {
        let r: DispatchResult = serde_json::from_str(r#"{"status":"passed"}"#)
            .expect("result parses");
        assert_eq!(r.status, Status::Passed);
        assert!(r.vars.is_empty());
        assert!(r.diagnostics.is_empty());
        assert!(r.error.is_none());
    }

    #[test]
    fn diagnostics_render_into_the_failure_text() {
        // The report layer composes a file's whole output into ONE string, so
        // plugin evidence has to arrive as text on the error path, not as a
        // separate print (invariant 6 + the atomic per-file dump).
        let r = DispatchResult {
            status: Status::Fatal,
            vars: Default::default(),
            diagnostics: vec![
                Diagnostic { title: "PUT /b/o".into(), kind: "http".into(),
                             content: Some("403 Forbidden".into()), path: None },
                Diagnostic { title: "Screenshot".into(), kind: "image".into(),
                             content: None, path: Some("/run/artifacts/7/fail.png".into()) },
            ],
            error: Some("access denied".into()),
        };
        let text = r.render_failure();
        assert!(text.contains("access denied"), "{text}");
        assert!(text.contains("PUT /b/o"), "{text}");
        assert!(text.contains("403 Forbidden"), "{text}");
        assert!(text.contains("/run/artifacts/7/fail.png"), "{text}");
    }

    #[test]
    fn an_envelope_carries_the_error() {
        let e: Envelope = serde_json::from_str(r#"{"ok":false,"error":"bucket is required"}"#)
            .expect("envelope parses");
        assert_eq!(e.into_result().unwrap_err(), "bucket is required");
    }

    #[test]
    fn an_envelope_without_a_message_still_fails() {
        // A plugin that returns ok:false with no message must not be reported
        // as a success, and must not panic the host either.
        let e: Envelope = serde_json::from_str(r#"{"ok":false}"#).expect("envelope parses");
        assert!(e.into_result().is_err());
    }

    #[test]
    fn an_init_response_yields_a_handle() {
        let r: InitResponse = serde_json::from_str(r#"{"ok":true,"handle":7}"#)
            .expect("init response parses");
        assert_eq!(r.into_result().expect("ok"), 7);
    }

    #[test]
    fn an_init_response_ok_with_no_handle_is_an_error() {
        // A plugin that forgets `handle` must fail loudly at init, not hand
        // back handle 0 and let a later step fail confusingly instead.
        let r: InitResponse =
            serde_json::from_str(r#"{"ok":true}"#).expect("init response parses");
        let err = r.into_result().unwrap_err();
        assert!(err.contains("handle"), "{err}");
    }

    #[test]
    fn a_dispatch_request_serialises_the_documented_keys() {
        // These key names are the half of the contract plugin authors parse:
        // renaming one is a breaking change no deserialise-side test would catch.
        let args = vec!["report.pdf".to_string()];
        let request = DispatchRequest {
            args: &args,
            docstring: None,
            table: None,
            artifacts_dir: "/tmp/run/0007".into(),
            debug: false,
            options: OptionsJson::from(&crate::options::Options::default()),
        };
        let value = serde_json::to_value(&request).expect("serialises");
        // `serde_json::Value::Object` is a `BTreeMap` (no `preserve_order`
        // feature here), so keys come back alphabetical, not in field order —
        // sort both sides to pin the key SET without depending on that.
        let mut keys: Vec<&String> = value.as_object().expect("an object").keys().collect();
        keys.sort();
        let mut expected = ["args", "docstring", "table", "artifacts_dir", "debug", "options"];
        expected.sort();
        assert_eq!(keys, expected);
        assert!(
            value["docstring"].is_null(),
            "an absent docstring stays present as null, as the documented payload shows"
        );
        assert_eq!(value["options"]["polling"]["interval_ms"], 100);
    }

    #[test]
    fn an_init_request_serialises_the_documented_keys_and_config_verbatim() {
        let config = serde_json::json!({"bucket": "acme-uploads", "region": "eu-west-1"});
        let request = InitRequest {
            group: "widget",
            instance: "primary",
            config: &config,
            options: OptionsJson::from(&crate::options::Options::default()),
        };
        let value = serde_json::to_value(&request).expect("serialises");
        let mut keys: Vec<&String> = value.as_object().expect("an object").keys().collect();
        keys.sort();
        let mut expected = ["group", "instance", "config", "options"];
        expected.sort();
        assert_eq!(keys, expected);
        assert_eq!(value["config"], config);
    }

    #[test]
    fn an_unknown_field_from_a_newer_plugin_is_ignored() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"name":"widget","version":"1.0.0","groups":["widget"],"capabilities":["streaming"]}"#,
        )
        .expect("a newer plugin's extra keys do not break an older host");
        assert_eq!(manifest.name, "widget");
    }
}
