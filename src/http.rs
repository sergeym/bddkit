use crate::hawk;
use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Exchange {
    pub method: String,
    pub url: String,
    pub req_headers: Vec<(String, String)>,
    pub req_body: Option<String>,
    pub status: u16,
    pub resp_headers: Vec<(String, String)>,
    pub body: String,
}

impl Exchange {
    pub fn json(&self) -> Result<Value, String> {
        serde_json::from_str(&self.body)
            .map_err(|e| format!("response body is not valid JSON: {e}"))
    }

    /// Returns the cookie value from the `Set-Cookie` headers.
    pub fn set_cookie(&self, name: &str) -> Option<String> {
        for (h, v) in &self.resp_headers {
            if !h.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let pair = v.split(';').next().unwrap_or("");
            if let Some((k, val)) = pair.split_once('=')
                && k.trim() == name
            {
                return Some(val.trim().to_string());
            }
        }
        None
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "  {} {}", self.method, self.url)?;
        for (k, v) in &self.req_headers {
            writeln!(f, "    {k}: {v}")?;
        }
        if let Some(b) = &self.req_body {
            writeln!(f, "    {}", truncate(b, 600))?;
        }
        writeln!(f, "  ← {}", self.status)?;
        write!(f, "    {}", truncate(&self.body, 600))
    }
}

/// One API resource: its own client (and therefore its own timeout and
/// connection pool), a base address, and the headers every scenario starts with.
pub struct ApiResource {
    client: reqwest::Client,
    base_url: url::Url,
    default_headers: Vec<(String, String)>,
}

impl ApiResource {
    pub fn new(
        base_url: &str,
        timeout_secs: u64,
        default_headers: Vec<(String, String)>,
    ) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .context("failed to create HTTP client")?,
            base_url: url::Url::parse(base_url)
                .with_context(|| format!("invalid base_url {base_url:?}"))?,
            default_headers,
        })
    }
}

/// All of the run's API resources. Built once and shared via `Arc`: the
/// client holds a connection pool, and recreating it per scenario would lose the pool.
pub struct Apis {
    by_name: HashMap<String, ApiResource>,
    default: String,
}

impl Apis {
    /// `default: None` means "no APIs declared at all" (see `Config::resolve_default_api`) —
    /// legal for a DB-only config. An explicit `Some(default)` not named among
    /// the resources is an error: the rest of the code relies on the default
    /// being resolvable and shouldn't have to recheck it.
    pub fn new(by_name: HashMap<String, ApiResource>, default: Option<String>) -> Result<Self> {
        let default = match default {
            Some(d) if by_name.contains_key(&d) => d,
            Some(d) => anyhow::bail!("default API resource {d:?} is not declared in resources.api"),
            None => String::new(),
        };
        Ok(Self { by_name, default })
    }

    pub fn get(&self, name: &str) -> Result<&ApiResource, String> {
        self.by_name.get(name).ok_or_else(|| {
            if self.by_name.is_empty() {
                "no API resource is declared in the config (resources.api), \
                 but the scenario reaches for an API"
                    .to_string()
            } else {
                format!("API resource {name:?} is not declared in resources.api")
            }
        })
    }

    pub fn default_name(&self) -> &str {
        &self.default
    }

    fn default_headers(&self) -> Vec<(String, String)> {
        self.by_name
            .get(&self.default)
            .map(|r| r.default_headers.clone())
            .unwrap_or_default()
    }
}

pub struct HttpState {
    apis: Arc<Apis>,
    current: String,
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    body: Option<String>,
    form: Option<Vec<(String, String)>>,
    last: Option<Exchange>,
    /// One-shot Hawk credentials for the NEXT `send()` call only. Consumed
    /// (never persisted) inside `send`; cleared by API switch and scenario
    /// reset via `switch_to`.
    signer: Option<hawk::Credentials>,
}

impl HttpState {
    pub fn new(apis: Arc<Apis>) -> Self {
        let mut state = Self {
            apis,
            current: String::new(),
            headers: Vec::new(),
            query: Vec::new(),
            body: None,
            form: None,
            last: None,
            signer: None,
        };
        state.reset();
        state
    }

    /// Scenario boundary: the current resource, headers, and accumulated
    /// request return to their initial state; the last exchange is forgotten.
    pub fn reset(&mut self) {
        let default = self.apis.default_name().to_string();
        let headers = self.apis.default_headers();
        self.switch_to(default, headers);
        self.last = None;
    }

    /// Switches to another API. Headers are replaced with the new resource's
    /// default headers, and the accumulated request is cleared: auth set up
    /// for one host must not leak to another. `last` is preserved — it is a
    /// response, not a request, and checks against it must still work after switching.
    pub fn use_api(&mut self, name: &str) -> Result<(), String> {
        let headers = self.apis.get(name)?.default_headers.clone();
        self.switch_to(name.to_string(), headers);
        Ok(())
    }

    fn switch_to(&mut self, name: String, headers: Vec<(String, String)>) {
        self.current = name;
        self.headers = headers;
        self.query.clear();
        self.body = None;
        self.form = None;
        self.signer = None;
    }

    /// Sign the NEXT `send()` call with Hawk, and only that one. `send`
    /// consumes the signer, so a scenario must call this again before every
    /// request it wants signed.
    pub fn sign_next(&mut self, id: &str, key: &str) {
        self.signer = Some(hawk::Credentials {
            id: id.to_string(),
            key: key.to_string(),
        });
    }

    #[allow(dead_code)] // only read by tests, to assert the api switch
    pub fn current(&self) -> &str {
        &self.current
    }

    /// Replace: removes all values with this name, then sets one.
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
    }

    /// Appends to the header's existing values.
    pub fn add_header(&mut self, name: &str, value: &str) {
        self.headers.push((name.to_string(), value.to_string()));
    }

    pub fn set_query(&mut self, name: &str, value: &str) {
        self.query.retain(|(k, _)| k != name);
        self.query.push((name.to_string(), value.to_string()));
    }

    pub fn set_body(&mut self, body: String) {
        self.body = Some(body);
        self.form = None;
    }

    pub fn clear_body(&mut self) {
        self.body = None;
        self.form = None;
    }

    pub fn set_form(&mut self, pairs: Vec<(String, String)>) {
        self.form = Some(pairs);
        self.body = None;
    }

    pub fn last(&self) -> Option<&Exchange> {
        self.last.as_ref()
    }

    pub async fn send(&mut self, path: &str, method: &str) -> Result<(), String> {
        // Clear the last exchange BEFORE sending: on a network failure
        // (connection refused, timeout) no successful exchange gets recorded,
        // and a failure dump would otherwise show the previous step's stale
        // response as the failed one's.
        self.last = None;
        let api = self.apis.get(&self.current)?;
        let mut url = api
            .base_url
            .join(path)
            .map_err(|e| format!("invalid path {path:?}: {e}"))?;
        if !self.query.is_empty() {
            let mut qp = url.query_pairs_mut();
            for (k, v) in &self.query {
                qp.append_pair(k, v);
            }
        }
        let m = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| format!("invalid HTTP method {method:?}: {e}"))?;

        // Built before headers: a pending Hawk signer needs the exact bytes
        // being sent to hash the payload.
        let sent_body = if let Some(form) = &self.form {
            Some(
                form.iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            )
        } else {
            self.body.clone()
        };

        let mut request_headers = self.headers.clone();
        if let Some(credentials) = self.signer.take() {
            if self.form.is_some() {
                return Err(
                    "Hawk signing supports raw request bodies only, not form bodies".to_string(),
                );
            }
            let nonce = generate_nonce()?;
            let timestamp = unix_timestamp()?;
            request_headers = apply_signer(
                request_headers,
                &credentials,
                &url,
                method,
                sent_body.as_deref().unwrap_or(""),
                timestamp,
                &nonce,
            )?;
        }

        let mut req = api.client.request(m.clone(), url.clone());
        for (k, v) in &request_headers {
            req = req.header(k, v);
        }
        if let Some(form) = &self.form {
            req = req.form(form);
        } else if let Some(b) = &self.body {
            req = req.body(b.clone());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status().as_u16();
        let resp_headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<non-UTF8>").to_string()))
            .collect();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("failed to read response body: {e}"))?;

        self.last = Some(Exchange {
            method: m.to_string(),
            url: url.to_string(),
            req_headers: request_headers,
            req_body: sent_body,
            status,
            resp_headers,
            body,
        });
        Ok(())
    }
}

/// 16 random bytes, Base64-encoded, as a fresh Hawk nonce.
fn generate_nonce() -> Result<String, String> {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).map_err(|_| "failed to read system entropy".to_string())?;
    Ok(STANDARD.encode(buf))
}

/// Current Unix time in seconds, for the Hawk `ts` field.
fn unix_timestamp() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "system clock is set before the Unix epoch".to_string())
}

/// Signs a cloned header vector with Hawk: rejects an ambiguous multi-valued
/// `Content-Type`, then replaces any existing `Authorization` header
/// (case-insensitively) with the generated one. Takes `headers` by value so
/// the caller's own vector is never touched — only ever a clone is signed.
fn apply_signer(
    mut headers: Vec<(String, String)>,
    credentials: &hawk::Credentials,
    url: &url::Url,
    method: &str,
    body: &str,
    timestamp: u64,
    nonce: &str,
) -> Result<Vec<(String, String)>, String> {
    let content_types: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str())
        .collect();
    if content_types.len() > 1 {
        return Err(
            "multiple Content-Type headers: Hawk signing needs one unambiguous value".to_string(),
        );
    }
    let content_type = content_types.first().copied();

    let header = hawk::authorization(url, method, body, content_type, credentials, timestamp, nonce)?;
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
    headers.push(("Authorization".to_string(), header));
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn apis_with(name: &str, base: &str, default_headers: Vec<(String, String)>) -> Arc<Apis> {
        let mut by_name = HashMap::new();
        by_name.insert(
            name.to_string(),
            ApiResource::new(base, 5, default_headers).expect("valid base_url"),
        );
        Arc::new(Apis::new(by_name, Some(name.to_string())).expect("default declared"))
    }

    fn state() -> HttpState {
        HttpState::new(apis_with("main", "http://localhost:1/", Vec::new()))
    }

    fn two_apis() -> Arc<Apis> {
        let mut by_name = HashMap::new();
        by_name.insert(
            "first".to_string(),
            ApiResource::new(
                "http://first.local/",
                5,
                vec![("x-source".to_string(), "first".to_string())],
            )
            .expect("valid base_url"),
        );
        by_name.insert(
            "second".to_string(),
            ApiResource::new(
                "http://second.local/",
                5,
                vec![("x-source".to_string(), "second".to_string())],
            )
            .expect("valid base_url"),
        );
        Arc::new(Apis::new(by_name, Some("first".to_string())).expect("default declared"))
    }

    #[test]
    fn use_api_switches_the_current_resource() {
        let mut s = HttpState::new(two_apis());
        s.use_api("second").expect("resource is declared");
        assert_eq!(s.current(), "second");
    }

    #[test]
    fn use_api_replaces_headers_with_the_new_resource_defaults() {
        let mut s = HttpState::new(two_apis());
        s.set_header("authorization", "Bearer first-host");
        s.use_api("second").expect("resource is declared");
        assert_eq!(
            s.headers,
            vec![("x-source".to_string(), "second".to_string())]
        );
    }

    #[test]
    fn use_api_clears_the_pending_body() {
        let mut s = HttpState::new(two_apis());
        s.set_body("{\"a\":1}".to_string());
        s.use_api("second").expect("resource is declared");
        assert!(s.body.is_none());
    }

    #[test]
    fn use_api_clears_the_pending_query() {
        let mut s = HttpState::new(two_apis());
        s.set_query("page", "2");
        s.use_api("second").expect("resource is declared");
        assert!(s.query.is_empty());
    }

    #[test]
    fn use_api_keeps_the_last_exchange() {
        let mut s = HttpState::new(two_apis());
        s.last = Some(Exchange {
            method: "GET".into(),
            url: "http://first.local/x".into(),
            req_headers: vec![],
            req_body: None,
            status: 200,
            resp_headers: vec![],
            body: "{}".into(),
        });
        s.use_api("second").expect("resource is declared");
        assert!(
            s.last().is_some(),
            "the previous response must survive an API switch"
        );
    }

    #[test]
    fn use_api_with_an_undeclared_name_is_an_error() {
        let mut s = HttpState::new(two_apis());
        assert!(s.use_api("third").is_err());
    }

    #[test]
    fn reset_returns_to_the_default_resource() {
        let mut s = HttpState::new(two_apis());
        s.use_api("second").expect("resource is declared");
        s.reset();
        assert_eq!(s.current(), "first");
    }

    #[test]
    fn signing_is_cleared_when_an_api_is_switched() {
        let mut state = HttpState::new(two_apis());
        state.sign_next("id", "key");
        state.use_api("second").unwrap();
        assert!(state.signer.is_none());
    }

    #[test]
    fn signing_is_cleared_by_scenario_reset() {
        let mut state = state();
        state.sign_next("id", "key");
        state.reset();
        assert!(state.signer.is_none());
    }

    #[tokio::test]
    async fn a_signed_form_request_fails_before_transport() {
        let mut state = state();
        state.set_form(vec![("name".into(), "value".into())]);
        state.sign_next("id", "key");
        assert!(state.send("/x", "POST").await.unwrap_err().contains("form"));
    }

    #[test]
    fn signer_replaces_only_the_sent_authorization_header() {
        let mut s = state();
        s.set_header("authorization", "Bearer stale");
        let credentials = hawk::Credentials {
            id: "id".to_string(),
            key: "key".to_string(),
        };
        let url = url::Url::parse("http://localhost/x").unwrap();

        let signed = apply_signer(
            s.headers.clone(),
            &credentials,
            &url,
            "GET",
            "",
            1,
            "fixed-nonce",
        )
        .expect("signing succeeds");

        assert!(
            !signed
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer stale"),
            "stale Authorization must be replaced, not kept alongside the new one"
        );
        assert_eq!(
            signed
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                .count(),
            1
        );
        assert!(
            signed
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.starts_with("Hawk "))
        );
        // The state's own header vector must be untouched — apply_signer only
        // ever sees a clone.
        assert_eq!(
            s.headers,
            vec![("authorization".to_string(), "Bearer stale".to_string())]
        );
    }

    #[test]
    fn api_resource_rejects_an_invalid_base_url() {
        assert!(ApiResource::new("not-a-url", 5, Vec::new()).is_err());
    }

    #[test]
    fn apis_reject_a_default_naming_an_undeclared_resource() {
        let mut by_name = HashMap::new();
        by_name.insert(
            "a".to_string(),
            ApiResource::new("http://a.local/", 5, Vec::new()).expect("valid base_url"),
        );
        assert!(Apis::new(by_name, Some("b".to_string())).is_err());
    }

    #[test]
    fn apis_with_no_resources_and_no_default_construct_fine() {
        // Legal: a DB-only config with not a single API resource (see resolve_default_api).
        assert!(Apis::new(HashMap::new(), None).is_ok());
    }

    #[test]
    fn getting_a_resource_when_none_are_declared_names_the_missing_section() {
        let apis = Apis::new(HashMap::new(), None).expect("an empty set is not an error");
        let err = match apis.get("stub") {
            Ok(_) => panic!("there are no resources at all"),
            Err(e) => e,
        };
        assert!(err.contains("resources.api"), "{err}");
    }

    #[test]
    fn new_state_starts_on_the_default_resource() {
        let s = HttpState::new(apis_with("main", "http://localhost:1/", Vec::new()));
        assert_eq!(s.current(), "main");
    }

    #[test]
    fn new_state_seeds_headers_from_the_resource_defaults() {
        let defaults = vec![("accept".to_string(), "application/json".to_string())];
        let s = HttpState::new(apis_with("main", "http://localhost:1/", defaults));
        assert_eq!(
            s.headers,
            vec![("accept".to_string(), "application/json".to_string())]
        );
    }

    #[test]
    fn set_header_replaces_existing_case_insensitively() {
        let mut s = state();
        s.set_header("Accept", "text/plain");
        s.set_header("accept", "application/json");
        assert_eq!(s.headers.len(), 1);
        assert_eq!(s.headers[0].1, "application/json");
    }

    #[test]
    fn add_header_appends() {
        let mut s = state();
        s.set_header("Accept", "a");
        s.add_header("Accept", "b");
        assert_eq!(s.headers.len(), 2);
    }

    #[tokio::test]
    async fn failed_send_leaves_no_stale_exchange() {
        // Port 1 is unreachable: send fails at the transport layer. `last`
        // must stay None, or a failure dump would show a stale exchange.
        let mut s = state();
        assert!(s.send("/x", "GET").await.is_err());
        assert!(
            s.last().is_none(),
            "a failed send must not leave an exchange behind"
        );
    }

    #[test]
    fn body_and_form_are_mutually_exclusive() {
        let mut s = state();
        s.set_body("{}".into());
        s.set_form(vec![("a".into(), "1".into())]);
        assert!(s.body.is_none());
        s.set_body("{}".into());
        assert!(s.form.is_none());
    }

    #[test]
    fn extracts_cookie_from_set_cookie_header() {
        let ex = Exchange {
            method: "GET".into(),
            url: "http://x/".into(),
            req_headers: vec![],
            req_body: None,
            status: 200,
            resp_headers: vec![
                (
                    "set-cookie".into(),
                    "jwt_token=abc123; Path=/; HttpOnly".into(),
                ),
                ("set-cookie".into(), "refresh_token=def; Path=/".into()),
            ],
            body: String::new(),
        };
        assert_eq!(ex.set_cookie("jwt_token").as_deref(), Some("abc123"));
        assert_eq!(ex.set_cookie("refresh_token").as_deref(), Some("def"));
        assert_eq!(ex.set_cookie("absent"), None);
    }

    #[test]
    fn exchange_display_shows_request_and_response() {
        let ex = Exchange {
            method: "POST".into(),
            url: "http://x/login".into(),
            req_headers: vec![("Content-Type".into(), "application/json".into())],
            req_body: Some("{\"a\":1}".into()),
            status: 422,
            resp_headers: vec![],
            body: "{\"error\":\"no\"}".into(),
        };
        let s = ex.to_string();
        assert!(s.contains("POST http://x/login"));
        assert!(s.contains("Content-Type: application/json"));
        assert!(s.contains("← 422"));
    }
}
