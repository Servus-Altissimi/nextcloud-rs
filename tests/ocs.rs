//! The OCS envelope: mandatory header, and where the outcome is read from.

use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{client, ocs_body};

#[tokio::test]
async fn ocs_api_request_header() {
    let server = MockServer::start().await;

    // no OCS-APIRequest header and you get an HTML login page back
    Mock::given(method("GET"))
        .and(path("/ocs/v2.php/cloud/capabilities"))
        .and(header("OCS-APIRequest", "true"))
        .and(query_param("format", "json"))
        .respond_with(ocs_body(serde_json::json!({
            "version": {"major": 31, "minor": 0, "micro": 2, "string": "31.0.2"},
            "capabilities": {"core": {"pollinterval": 60}}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let caps = client(&server).capabilities().await.unwrap();
    assert_eq!(caps.version.major, 31);
    assert_eq!(caps.poll_interval(), Some(60));
}

#[tokio::test]
async fn ocs_failures_are_read_from_the_envelope_not_the_http_status() {
    let server = MockServer::start().await;

    // v1 reports not-found as HTTP 200 + statuscode 998
    Mock::given(method("GET"))
        .and(path("/ocs/v1.php/cloud/users/ghost"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            serde_json::json!({
                "ocs": {
                    "meta": {"status": "failure", "statuscode": 998, "message": "The requested user could not be found"},
                    "data": []
                }
            })
            .to_string(),
        ))
        .mount(&server)
        .await;

    let err = client(&server).users().get("ghost").await.unwrap_err();
    assert!(err.is_not_found(), "expected not-found, got {err:?}");
    assert!(err.to_string().contains("998"));
}

#[tokio::test]
async fn a_non_envelope_body_is_reported_clearly() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v2.php/cloud/capabilities"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("<!DOCTYPE html><html>login</html>"),
        )
        .mount(&server)
        .await;

    let err = client(&server).capabilities().await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not an OCS envelope"), "got: {msg}");
}
