//! Public helpers that no other test path reaches.

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{client, mount_ocs, ocs_body};
use nextcloud::{
    Capabilities, NewShare, PreviewMode, PreviewOptions, SharePermissions, ShareType, ShareUpdate,
    StatusType,
};

#[tokio::test]
async fn ocs_raw() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v2.php/apps/serverinfo/api/v1/info"))
        .and(query_param("format", "json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                serde_json::json!({
                    "ocs": {
                        "meta": {
                            "status": "ok",
                            "statuscode": 200,
                            "message": "OK",
                            "totalitems": "3",
                            "itemsperpage": 25
                        },
                        "data": {"nextcloud": {"system": {"version": "31.0.2"}}}
                    }
                })
                .to_string(),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let response = client(&server)
        .ocs_raw(
            reqwest::Method::GET,
            "ocs/v2.php/apps/serverinfo/api/v1/info",
            &[],
            &[],
        )
        .await
        .unwrap();

    assert_eq!(response.meta.total_items(), Some(3));
    assert_eq!(response.meta.items_per_page(), Some(25));
    assert_eq!(response.meta.status, "ok");
    assert_eq!(
        response.data["nextcloud"]["system"]["version"],
        serde_json::json!("31.0.2")
    );
}

#[tokio::test]
async fn public_preview_url() {
    let server = MockServer::start().await;
    let nc = client(&server);

    let url = nc
        .previews()
        .public_preview_url(
            "tok en",
            PreviewOptions::sized(64, 32).mode(PreviewMode::Fill),
        )
        .unwrap();

    // token is a path segment, so the space needs escaping
    assert!(
        url.path()
            .ends_with("/index.php/apps/files_sharing/publicpreview/tok%20en")
    );

    let query: Vec<_> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert!(query.contains(&("x".into(), "64".into())));
    assert!(query.contains(&("mode".into(), "fill".into())));
}

#[tokio::test]
async fn preview_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/index.php/core/preview"))
        .respond_with(ResponseTemplate::new(401).set_body_string("denied"))
        .mount(&server)
        .await;

    let nc = client(&server);
    let url = nc
        .previews()
        .url_for_file_id(1, PreviewOptions::default())
        .unwrap();

    let err = nc.previews().fetch(url).await.unwrap_err();
    assert!(err.is_auth_error(), "expected an auth error, got {err:?}");
}

#[test]
fn capability_lookups() {
    let caps: Capabilities = serde_json::from_value(serde_json::json!({
        "version": {"major": 31, "minor": 0, "micro": 2, "string": "31.0.2"},
        "capabilities": {
            "files_sharing": {
                "api_enabled": true,
                "numeric": 1,
                "off": 0,
                "worded": "yes",
                "nested": {"enabled": "1"},
                "listy": []
            }
        }
    }))
    .unwrap();

    assert_eq!(caps.get_bool("files_sharing", "api_enabled"), Some(true));
    assert_eq!(caps.get_bool("files_sharing", "numeric"), Some(true));
    assert_eq!(caps.get_bool("files_sharing", "off"), Some(false));
    assert_eq!(caps.get_bool("files_sharing", "worded"), Some(true));
    assert_eq!(caps.get_bool("files_sharing", "listy"), None);

    let app = caps.app("files_sharing").expect("the app block");
    assert_eq!(app["nested"]["enabled"], serde_json::json!("1"));
    assert!(caps.app("spreed").is_none());
}

#[test]
fn permission_bitmasks() {
    let mut perms = SharePermissions::READ;
    perms |= SharePermissions::UPDATE;
    assert_eq!(perms.bits(), 3);

    assert_eq!(SharePermissions::from_bits(3), perms);
    assert!(SharePermissions::ALL.contains(perms));
    assert!(!perms.contains(SharePermissions::DELETE));
}

#[test]
fn share_type_serde() {
    let json = serde_json::to_string(&ShareType::TalkConversation).unwrap();
    assert_eq!(json, "10");

    let back: ShareType = serde_json::from_str("10").unwrap();
    assert_eq!(back, ShareType::TalkConversation);

    assert!(!ShareType::PublicLink.needs_recipient());
    assert!(ShareType::Circle.needs_recipient());
    assert!(ShareType::FederatedCloud.needs_recipient());
}

#[test]
fn empty_update() {
    assert!(ShareUpdate::new().is_empty());
    assert!(ShareUpdate::default().is_empty());
    assert!(!ShareUpdate::new().note("x").is_empty());
}

#[test]
fn arbitrary_share_type() {
    let share = NewShare::new("/a.txt", ShareType::Other(99)).share_with("x");
    let debug = format!("{share:?}");
    assert!(debug.contains("Other(99)"), "got {debug}");
}

#[test]
fn status_type_parsing() {
    use std::str::FromStr;

    assert_eq!(
        StatusType::from_str("dnd").unwrap(),
        StatusType::DoNotDisturb
    );
    assert_eq!(
        StatusType::from_str("invisible").unwrap(),
        StatusType::Invisible
    );
    assert_eq!(
        StatusType::from_str("brb").unwrap(),
        StatusType::Other("brb".into())
    );
}

#[tokio::test]
async fn anonymous_capabilities() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v2.php/cloud/capabilities",
        serde_json::json!({
            "version": {"major": 30, "minor": 4, "micro": 1, "string": "30.4.1"},
            "capabilities": {"core": {"webdav-root": "remote.php/webdav"}}
        }),
    )
    .await;

    let nc = nextcloud::Nextcloud::builder(server.uri()).build().unwrap();
    let caps = nc.capabilities().await.unwrap();

    assert!(caps.version.at_least(30, 4));
    assert!(!caps.version.at_least(31, 0));
    assert_eq!(caps.webdav_root(), Some("remote.php/webdav"));
    assert_eq!(caps.poll_interval(), None);
}

#[tokio::test]
async fn pagination_counters() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v1.php/cloud/users"))
        .respond_with(ocs_body(serde_json::json!({
            "users": ["alice"]
        })))
        .mount(&server)
        .await;

    let raw = client(&server)
        .ocs_raw(reqwest::Method::GET, "ocs/v1.php/cloud/users", &[], &[])
        .await
        .unwrap();
    assert!(raw.meta.is_success());
    assert_eq!(raw.meta.total_items(), None);
}

#[tokio::test]
async fn missing_preview() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/index.php/core/preview"))
        .respond_with(ResponseTemplate::new(404).set_body_string("{}"))
        .mount(&server)
        .await;

    let nc = client(&server);
    let url = nc
        .previews()
        .url_for_file_id(42, nextcloud::PreviewOptions::square(256))
        .unwrap();

    assert!(nc.previews().fetch(url).await.unwrap().is_none());
}

#[tokio::test]
async fn preview_content_type() {
    let server = MockServer::start().await;

    // previews are not labelled reliably, so sniff the bytes
    Mock::given(method("GET"))
        .and(path("/index.php/core/preview"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nrest".to_vec()))
        .mount(&server)
        .await;

    let nc = client(&server);
    let url = nc
        .previews()
        .url_for_file_id(42, nextcloud::PreviewOptions::square(256))
        .unwrap();

    let preview = nc.previews().fetch(url).await.unwrap().expect("a preview");
    assert_eq!(preview.content_type, "image/png");
    assert!(preview.bytes.starts_with(b"\x89PNG"));
}
