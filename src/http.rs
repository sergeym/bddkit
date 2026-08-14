use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;

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

pub struct HttpState {
    client: reqwest::Client,
    base_url: url::Url,
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    body: Option<String>,
    form: Option<Vec<(String, String)>>,
    last: Option<Exchange>,
}

impl HttpState {
    pub fn new(base_url: &str, timeout_secs: u64) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .context("failed to create HTTP client")?,
            base_url: url::Url::parse(base_url)
                .with_context(|| format!("invalid base_url {base_url:?}"))?,
            headers: Vec::new(),
            query: Vec::new(),
            body: None,
            form: None,
            last: None,
        })
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
        // Reset the previous exchange BEFORE sending: on a network failure (connection
        // refused, timeout) no successful exchange is recorded, and a failure dump would
        // otherwise show a stale response from a previous step as the failed one's.
        self.last = None;
        let mut url = self
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

        let mut req = self.client.request(m.clone(), url.clone());
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let sent_body = if let Some(form) = &self.form {
            req = req.form(form);
            Some(
                form.iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            )
        } else if let Some(b) = &self.body {
            req = req.body(b.clone());
            Some(b.clone())
        } else {
            None
        };

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
            req_headers: self.headers.clone(),
            req_body: sent_body,
            status,
            resp_headers,
            body,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> HttpState {
        HttpState::new("http://localhost:1/", 5).unwrap()
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
        // Port 1 is unreachable: send fails at the transport level. `last` must stay
        // None, or a failure dump would show a stale exchange.
        let mut s = state();
        assert!(s.send("/x", "GET").await.is_err());
        assert!(s.last().is_none(), "a failed send must not leave a stale exchange");
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

    #[test]
    fn invalid_base_url_is_rejected() {
        assert!(HttpState::new("not-a-url", 5).is_err());
    }
}
