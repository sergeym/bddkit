//! Reference HTTP service for acceptance tests. Spun up in-process
//! on a random port: docker-compose arrives in M2 together with Postgres.

#![allow(dead_code)]

pub mod db;

use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Barrier;

pub async fn spawn() -> String {
    let app = Router::new()
        .route("/ping", get(ping))
        .route("/echo", post(echo))
        .route("/users", get(users))
        .route("/login", post(login))
        .route("/headers", get(headers))
        .route("/xml", get(xml_doc))
        .route("/html", get(html_doc))
        .route("/plain", get(plain_text));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}/")
}

/// A second reference service. Answers the same `/ping`, but with a different body:
/// the API-switch test must see that the request landed here specifically.
pub async fn spawn_secondary() -> String {
    let app = Router::new().route(
        "/ping",
        get(|| async { Json(json!({"status": "ok", "source": "secondary"})) }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}/")
}

/// A stub with a barrier: `/barrier` does not respond until `n` requests are
/// in flight at once. A sequential runner hangs on it until timeout,
/// a parallel one gets through — this proves parallelism without comparing run
/// times, i.e. without a flaky test.
pub async fn spawn_barrier(n: usize) -> String {
    let barrier = Arc::new(Barrier::new(n));
    let app = Router::new().route(
        "/barrier",
        get(move || {
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                Json(json!({"status": "ok"}))
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}/")
}

async fn ping() -> Json<Value> {
    Json(json!({"status": "ok", "version": 3}))
}

async fn echo(body: String) -> impl IntoResponse {
    let parsed: Value = serde_json::from_str(&body).unwrap_or(json!({}));
    (
        StatusCode::CREATED,
        Json(json!({"received": parsed, "id": 42})),
    )
}

async fn users(Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    if q.get("email").is_some_and(|e| e == "a@b.net") {
        return Json(json!([{"id": 1, "email": "a@b.net"}]));
    }
    Json(json!([
        {"id": 1, "email": "a@b.net", "roles": ["ROLE_USER"]},
        {"id": 2, "email": "c@d.net", "roles": ["ROLE_ADMIN"]}
    ]))
}

async fn login(body: String) -> impl IntoResponse {
    let parsed: Value = serde_json::from_str(&body).unwrap_or(json!({}));
    if parsed.get("password").and_then(Value::as_str) == Some("correct") {
        let mut h = HeaderMap::new();
        h.insert(
            "set-cookie",
            "jwt_token=tok-abc; Path=/; HttpOnly"
                .parse()
                .expect("header"),
        );
        h.insert("x-trace", "trace-1".parse().expect("header"));
        return (
            StatusCode::OK,
            h,
            Json(json!({"success": true, "jwt": "tok-abc"})),
        );
    }
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        HeaderMap::new(),
        Json(json!({"success": false, "errorMessage": "Wrong password"})),
    )
}

async fn headers(h: HeaderMap) -> Json<Value> {
    let accept: Vec<String> = h
        .get_all("accept")
        .iter()
        .map(|v| v.to_str().unwrap_or("").to_string())
        .collect();
    Json(json!({"accept": accept}))
}

async fn xml_doc() -> impl IntoResponse {
    let mut h = HeaderMap::new();
    h.insert("content-type", "application/xml".parse().expect("header"));
    (
        StatusCode::OK,
        h,
        r#"<?xml version="1.0"?><users><user id="1"><email>a@b.net</email></user><user id="2"><email>c@d.net</email></user></users>"#,
    )
}

async fn html_doc() -> impl IntoResponse {
    let mut h = HeaderMap::new();
    h.insert("content-type", "text/html".parse().expect("header"));
    (
        StatusCode::OK,
        h,
        r#"<html><body><h1 id="title">Hello</h1><p class="msg">World</p></body></html>"#,
    )
}

async fn plain_text() -> impl IntoResponse {
    let mut h = HeaderMap::new();
    h.insert("content-type", "text/plain".parse().expect("header"));
    (StatusCode::OK, h, "hello")
}
