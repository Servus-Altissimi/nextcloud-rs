//! Shared setup for the mock-server tests.

#![allow(dead_code)]

use nextcloud::Nextcloud;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn client(server: &MockServer) -> Nextcloud {
    Nextcloud::builder(server.uri())
        .basic_auth("alice", "app-password")
        .build()
        .unwrap()
}

/// Wrap `data` in a successful OCS envelope.
pub fn ocs(data: serde_json::Value) -> String {
    serde_json::json!({
        "ocs": {
            "meta": {"status": "ok", "statuscode": 200, "message": "OK"},
            "data": data
        }
    })
    .to_string()
}

/// Wrap `data` in a failing OCS envelope carrying `code`.
pub fn ocs_failure(code: i32, message: &str) -> String {
    serde_json::json!({
        "ocs": {
            "meta": {"status": "failure", "statuscode": code, "message": message},
            "data": []
        }
    })
    .to_string()
}

/// Wrap `responses` in a multistatus document declaring the namespaces a real
/// server sends.
pub fn multistatus(responses: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">{responses}</d:multistatus>"#
    )
}

/// One successful `<d:response>`, with `props` given as raw property elements.
pub fn dav_response(href: &str, props: &str) -> String {
    format!(
        r#"<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#
    )
}

/// A 200 carrying `data` in a successful OCS envelope.
pub fn ocs_body(data: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_string(ocs(data))
}

/// A 200 carrying an empty OCS payload, which is what the mutating endpoints
/// answer with.
pub fn ocs_ok() -> ResponseTemplate {
    ocs_body(serde_json::json!({}))
}

/// Expect exactly one `verb` request to `route`, answering with an empty
/// payload. A mutating call landing on the wrong route fails the test.
pub async fn mount_call(server: &MockServer, verb: &'static str, route: &'static str) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ocs_ok())
        .expect(1)
        .mount(server)
        .await;
}

/// As [`mount_call`], and the form body must contain `body_contains`.
pub async fn mount_form(
    server: &MockServer,
    verb: &'static str,
    route: &'static str,
    body_contains: &'static str,
) {
    Mock::given(method(verb))
        .and(path(route))
        .and(body_string_contains(body_contains))
        .respond_with(ocs_ok())
        .expect(1)
        .mount(server)
        .await;
}

/// Expect exactly one `verb` request to `dav_path`, answering with `code` and
/// no body.
pub async fn mount_status(
    server: &MockServer,
    verb: &'static str,
    dav_path: &'static str,
    code: u16,
) {
    Mock::given(method(verb))
        .and(path(dav_path))
        .respond_with(ResponseTemplate::new(code))
        .expect(1)
        .mount(server)
        .await;
}

pub async fn mount_propfind(server: &MockServer, dav_path: &'static str, body: String) {
    Mock::given(method("PROPFIND"))
        .and(path(dav_path))
        .respond_with(ResponseTemplate::new(207).set_body_string(body))
        .mount(server)
        .await;
}

/// Mount an OCS endpoint answering `data` for `verb` on `ocs_path`.
pub async fn mount_ocs(
    server: &MockServer,
    verb: &'static str,
    ocs_path: &'static str,
    data: serde_json::Value,
) {
    Mock::given(method(verb))
        .and(path(ocs_path))
        .respond_with(ocs_body(data))
        .mount(server)
        .await;
}
