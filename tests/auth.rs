//! Login Flow v2 polling behaviour and client construction.

use std::time::Duration;

use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

use nextcloud::{Credentials, Error, LoginCredentials, LoginFlowV2, Nextcloud};

async fn mount_init(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/index.php/login/v2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                serde_json::json!({
                    "poll": {
                        "token": "poll-token",
                        "endpoint": format!("{}/login/v2/poll", server.uri())
                    },
                    "login": format!("{}/login/v2/flow/abc", server.uri())
                })
                .to_string(),
            ),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn poll_timeout() {
    let server = MockServer::start().await;
    mount_init(&server).await;

    Mock::given(method("POST"))
        .and(path("/login/v2/poll"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let flow = LoginFlowV2::start(&server.uri()).await.unwrap();
    assert_eq!(flow.poll_token(), "poll-token");

    let err = flow
        .wait_with(Duration::from_millis(1), Duration::from_millis(3))
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::LoginFlowTimeout),
        "expected a timeout, got {err:?}"
    );
}

#[tokio::test]
async fn poll_unexpected_status() {
    let server = MockServer::start().await;
    mount_init(&server).await;

    // a 500 is not "still waiting"
    Mock::given(method("POST"))
        .and(path("/login/v2/poll"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let flow = LoginFlowV2::start(&server.uri()).await.unwrap();
    let err = flow
        .wait_with(Duration::from_millis(1), Duration::from_secs(5))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("HTTP 500"), "got {err}");
}

#[tokio::test]
async fn init_rejected() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/index.php/login/v2"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = LoginFlowV2::start(&server.uri()).await.unwrap_err();
    assert!(matches!(err, Error::Http(_)), "got {err:?}");
}

#[tokio::test]
async fn custom_client_used_for_login() {
    let server = MockServer::start().await;
    mount_init(&server).await;

    let http = reqwest::Client::builder()
        .user_agent("my-app/1.0")
        .build()
        .unwrap();

    let flow = LoginFlowV2::start_with_client(&server.uri(), http)
        .await
        .unwrap();
    assert!(flow.login_url().contains("/login/v2/flow/abc"));
}

#[test]
fn login_creds_to_basic() {
    let creds = LoginCredentials {
        server: "https://cloud.example.com".into(),
        login_name: "alice".into(),
        app_password: "generated".into(),
    };

    let converted = Credentials::from(creds.clone());
    assert_eq!(converted.login_name(), Some("alice"));

    let nc = Nextcloud::from_login(creds).unwrap();
    assert_eq!(nc.user_id(), Some("alice"));
}

#[test]
fn no_login_name() {
    assert_eq!(Credentials::default().login_name(), None);
    assert_eq!(Credentials::bearer("token").login_name(), None);
    assert_eq!(Credentials::basic("bob", "pw").login_name(), Some("bob"));
}

#[test]
fn builder_settings() {
    let nc = Nextcloud::builder("cloud.example.com")
        .basic_auth("alice", "pw")
        .user_id("alice.smith")
        .user_agent("my-app/1.0")
        .timeout(None)
        .build()
        .unwrap();

    assert_eq!(nc.user_id(), Some("alice.smith"));
    assert_eq!(nc.base_url().as_str(), "https://cloud.example.com/");
}

#[test]
fn builder_keeps_http_client() {
    let http = reqwest::Client::builder()
        .user_agent("my-app/1.0")
        .build()
        .unwrap();

    let nc = Nextcloud::builder("cloud.example.com")
        .credentials(Credentials::bearer("token"))
        .http_client(http)
        .build()
        .unwrap();

    assert!(format!("{:?}", nc.http_client()).contains("Client"));
    assert_eq!(nc.user_id(), None);
}

#[test]
fn set_user_id() {
    let mut nc = Nextcloud::builder("cloud.example.com").build().unwrap();
    assert_eq!(nc.user_id(), None);

    nc.set_user_id("carol");
    assert_eq!(nc.user_id(), Some("carol"));
}

#[test]
fn empty_server_url() {
    let err = Nextcloud::builder("   ").build().unwrap_err();
    assert!(err.to_string().contains("empty server URL"), "got {err}");
}

#[tokio::test]
async fn login_flow_v2() {
    let server = MockServer::start().await;
    mount_init(&server).await;

    // 404 = still waiting
    Mock::given(method("POST"))
        .and(path("/login/v2/poll"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/login/v2/poll"))
        .and(body_string_contains("token=poll-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                serde_json::json!({
                    "server": "https://cloud.example.com",
                    "loginName": "alice",
                    "appPassword": "generated-app-password"
                })
                .to_string(),
            ),
        )
        .mount(&server)
        .await;

    let flow = LoginFlowV2::start(&server.uri()).await.unwrap();
    assert!(flow.login_url().contains("/login/v2/flow/"));

    assert!(matches!(
        flow.poll_once().await,
        Err(Error::LoginFlowPending)
    ));

    let creds = flow
        .wait_with(Duration::from_millis(10), Duration::from_secs(5))
        .await
        .unwrap();

    assert_eq!(creds.login_name, "alice");
    assert_eq!(creds.app_password, "generated-app-password");

    let nc = Nextcloud::from_login(creds).unwrap();
    assert_eq!(nc.user_id(), Some("alice"));
    assert_eq!(nc.base_url().as_str(), "https://cloud.example.com/");
}
