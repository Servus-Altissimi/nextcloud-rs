//! Sharing, user status and notification wiring.

use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{client, mount_call, mount_ocs, ocs_body, ocs_failure};
use nextcloud::{NewShare, SharePermissions, ShareType, ShareUpdate, StatusType};

const SHARES: &str = "/ocs/v2.php/apps/files_sharing/api/v1/shares";

fn share_json(id: &str, share_type: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "share_type": share_type,
        "permissions": 1,
        "uid_owner": "alice",
        "path": "/report.pdf",
        "stime": 1700000000
    })
}

#[tokio::test]
async fn list_shares() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        SHARES,
        serde_json::json!([share_json("1", 0), share_json("2", 3)]),
    )
    .await;

    let shares = client(&server).shares().list().await.unwrap();
    assert_eq!(shares.len(), 2);
    assert_eq!(shares[0].share_type, ShareType::User);
    assert!(shares[1].is_public_link());
    assert_eq!(shares[0].created_at().unwrap().timestamp(), 1700000000);
}

#[tokio::test]
async fn list_shares_for_path() {
    let server = MockServer::start().await;

    // words, not 1/0: the numeric form is rejected
    Mock::given(method("GET"))
        .and(path(SHARES))
        .and(query_param("path", "/Documents"))
        .and(query_param("reshares", "true"))
        .and(query_param("subfiles", "false"))
        .respond_with(ocs_body(serde_json::json!([share_json("1", 0)])))
        .expect(1)
        .mount(&server)
        .await;

    let shares = client(&server)
        .shares()
        .list_for_path("/Documents", true, false)
        .await
        .unwrap();
    assert_eq!(shares.len(), 1);
}

#[tokio::test]
async fn list_shared_with_me() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SHARES))
        .and(query_param("shared_with_me", "true"))
        .respond_with(ocs_body(serde_json::json!([share_json("9", 0)])))
        .expect(1)
        .mount(&server)
        .await;

    let shares = client(&server)
        .shares()
        .list_shared_with_me()
        .await
        .unwrap();
    assert_eq!(shares[0].id, "9");
}

#[tokio::test]
async fn get_share() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v2.php/apps/files_sharing/api/v1/shares/17",
        serde_json::json!([share_json("17", 3)]),
    )
    .await;

    let share = client(&server).shares().get("17").await.unwrap();
    assert_eq!(share.id, "17");
}

#[tokio::test]
async fn get_share_empty_list() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v2.php/apps/files_sharing/api/v1/shares/17",
        serde_json::json!([]),
    )
    .await;

    let err = client(&server).shares().get("17").await.unwrap_err();
    assert!(
        err.to_string().contains("share 17 returned no entries"),
        "expected the empty-list message, got {err}"
    );
}

#[tokio::test]
async fn create_user_share() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(SHARES))
        .and(body_string_contains("shareType=0"))
        .and(body_string_contains("shareWith=bob"))
        .and(body_string_contains("permissions=3"))
        .respond_with(ocs_body(share_json("21", 0)))
        .expect(1)
        .mount(&server)
        .await;

    let share = client(&server)
        .shares()
        .create(
            NewShare::with_user("/report.pdf", "bob")
                .permissions(SharePermissions::READ | SharePermissions::UPDATE),
        )
        .await
        .unwrap();
    assert_eq!(share.id, "21");
}

#[tokio::test]
async fn create_group_and_email_share() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(SHARES))
        .and(body_string_contains("shareType=1"))
        .and(body_string_contains("shareWith=staff"))
        .respond_with(ocs_body(share_json("1", 1)))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(SHARES))
        .and(body_string_contains("shareType=4"))
        .and(body_string_contains("sendMail=true"))
        .respond_with(ocs_body(share_json("2", 4)))
        .expect(1)
        .mount(&server)
        .await;

    let nc = client(&server);
    nc.shares()
        .create(NewShare::with_group("/report.pdf", "staff"))
        .await
        .unwrap();
    nc.shares()
        .create(NewShare::with_email("/report.pdf", "bob@example.org").send_mail(true))
        .await
        .unwrap();
}

#[tokio::test]
async fn create_link_share() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(SHARES))
        .and(body_string_contains("note=for+review"))
        .and(body_string_contains("label=internal"))
        .and(body_string_contains("sendPasswordByTalk=true"))
        .and(body_string_contains("publicUpload=true"))
        .and(body_string_contains("expireDate=2026-12-31"))
        .and(body_string_contains("attributes="))
        .respond_with(ocs_body(share_json("3", 3)))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .shares()
        .create(
            NewShare::public_link("/Documents")
                .note("for review")
                .label("internal")
                .password("hunter2")
                .send_password_by_talk(true)
                .public_upload(true)
                .expire_date("2026-12-31")
                .attributes(r#"[{"scope":"permissions","key":"download","enabled":false}]"#),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn update_share() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/ocs/v2.php/apps/files_sharing/api/v1/shares/17"))
        .and(body_string_contains("permissions=31"))
        .and(body_string_contains("hideDownload=true"))
        .respond_with(ocs_body(share_json("17", 3)))
        .expect(1)
        .mount(&server)
        .await;

    let update = ShareUpdate::new()
        .permissions(SharePermissions::ALL)
        .hide_download(true);
    assert!(!update.is_empty());

    let share = client(&server).shares().update("17", update).await.unwrap();
    assert_eq!(share.id, "17");
}

#[tokio::test]
async fn update_share_clear_password() {
    let server = MockServer::start().await;

    // omitted means "leave alone", so clearing still sends the key
    Mock::given(method("PUT"))
        .and(path("/ocs/v2.php/apps/files_sharing/api/v1/shares/17"))
        .and(body_string_contains("password="))
        .and(body_string_contains("expireDate="))
        .respond_with(ocs_body(share_json("17", 3)))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .shares()
        .update(
            "17",
            ShareUpdate::new().clear_password().clear_expire_date(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn update_share_rejected() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/ocs/v2.php/apps/files_sharing/api/v1/shares/17"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(ocs_failure(400, "Password is too weak")),
        )
        .mount(&server)
        .await;

    let err = client(&server)
        .shares()
        .update("17", ShareUpdate::new().password("x"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Password is too weak"));
}

#[tokio::test]
async fn delete_and_mail_share() {
    let server = MockServer::start().await;

    mount_call(
        &server,
        "DELETE",
        "/ocs/v2.php/apps/files_sharing/api/v1/shares/17",
    )
    .await;
    mount_call(
        &server,
        "POST",
        "/ocs/v2.php/apps/files_sharing/api/v1/shares/17/send-email",
    )
    .await;

    let nc = client(&server);
    nc.shares().send_email("17").await.unwrap();
    nc.shares().delete("17").await.unwrap();
}

#[tokio::test]
async fn unknown_share_type_round_trip() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        SHARES,
        serde_json::json!([share_json("1", 42)]),
    )
    .await;

    let shares = client(&server).shares().list().await.unwrap();
    assert_eq!(shares[0].share_type, ShareType::Other(42));
    assert!(!shares[0].is_public_link());
}

const OWN_STATUS: &str = "/ocs/v2.php/apps/user_status/api/v1/user_status";

fn status_json(status: &str) -> serde_json::Value {
    serde_json::json!({
        "userId": "alice",
        "status": status,
        "message": "In a meeting",
        "clearAt": 1700000000
    })
}

#[tokio::test]
async fn own_status() {
    let server = MockServer::start().await;

    mount_ocs(&server, "GET", OWN_STATUS, status_json("dnd")).await;

    let status = client(&server).user_status().get_own().await.unwrap();
    assert_eq!(status.status, StatusType::DoNotDisturb);
    assert_eq!(status.clear_at_utc().unwrap().timestamp(), 1700000000);
}

#[tokio::test]
async fn set_availability() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path(
            "/ocs/v2.php/apps/user_status/api/v1/user_status/status",
        ))
        .and(body_string_contains("statusType=away"))
        .respond_with(ocs_body(status_json("away")))
        .expect(1)
        .mount(&server)
        .await;

    let status = client(&server)
        .user_status()
        .set_status(StatusType::Away)
        .await
        .unwrap();
    assert_eq!(status.status, StatusType::Away);
}

#[tokio::test]
async fn unknown_status_round_trip() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path(
            "/ocs/v2.php/apps/user_status/api/v1/user_status/status",
        ))
        .and(body_string_contains("statusType=vacationing"))
        .respond_with(ocs_body(status_json("vacationing")))
        .expect(1)
        .mount(&server)
        .await;

    let status = client(&server)
        .user_status()
        .set_status(StatusType::Other("vacationing".into()))
        .await
        .unwrap();
    assert_eq!(status.status, StatusType::Other("vacationing".into()));
}

#[tokio::test]
async fn predefined_message() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path(
            "/ocs/v2.php/apps/user_status/api/v1/user_status/message/predefined",
        ))
        .and(body_string_contains("messageId=meeting"))
        .and(body_string_contains("clearAt=1700000000"))
        .respond_with(ocs_body(status_json("dnd")))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .user_status()
        .set_predefined_message("meeting", Some(1700000000))
        .await
        .unwrap();
}

#[tokio::test]
async fn custom_message() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path(
            "/ocs/v2.php/apps/user_status/api/v1/user_status/message/custom",
        ))
        .and(body_string_contains("message=Back+soon"))
        .respond_with(ocs_body(status_json("online")))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .user_status()
        .set_custom_message(Some("Back soon"), None, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn clear_message() {
    let server = MockServer::start().await;

    mount_call(
        &server,
        "DELETE",
        "/ocs/v2.php/apps/user_status/api/v1/user_status/message",
    )
    .await;

    client(&server).user_status().clear_message().await.unwrap();
}

#[tokio::test]
async fn other_user_statuses() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v2.php/apps/user_status/api/v1/statuses"))
        .and(query_param("limit", "5"))
        .and(query_param("offset", "10"))
        .respond_with(ocs_body(serde_json::json!([status_json("online")])))
        .expect(1)
        .mount(&server)
        .await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v2.php/apps/user_status/api/v1/statuses/bob",
        status_json("offline"),
    )
    .await;

    let nc = client(&server);
    let listed = nc.user_status().list(Some(5), Some(10)).await.unwrap();
    assert_eq!(listed[0].status, StatusType::Online);

    let bob = nc.user_status().get("bob").await.unwrap();
    assert_eq!(bob.status, StatusType::Offline);
}

const NOTIFICATIONS: &str = "/ocs/v2.php/apps/notifications/api/v2/notifications";

#[tokio::test]
async fn list_notifications() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        NOTIFICATIONS,
        serde_json::json!([{
            "notification_id": 42,
            "app": "files_sharing",
            "user": "alice",
            "datetime": "2026-08-11T10:30:00+00:00",
            "subject": "Bob shared a folder with you",
            "actions": [
                {"label": "Accept", "link": "https://x/accept", "type": "POST", "primary": true},
                {"label": "Open", "link": "https://x", "type": "WEB", "primary": false}
            ]
        }]),
    )
    .await;

    let items = client(&server).notifications().list().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].primary_action().unwrap().label, "Accept");
    assert!(items[0].actions[1].is_web_link());
    assert_eq!(items[0].published_at().unwrap().timestamp(), 1786444200);
}

#[tokio::test]
async fn get_notification() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v2.php/apps/notifications/api/v2/notifications/42",
        serde_json::json!({
            "notification_id": 42,
            "app": "core",
            "subject": "Scan finished",
            "message": ""
        }),
    )
    .await;

    let item = client(&server).notifications().get(42).await.unwrap();
    assert_eq!(item.notification_id, 42);
    assert_eq!(item.message, None);
}

#[tokio::test]
async fn dismiss_notifications() {
    let server = MockServer::start().await;

    mount_call(
        &server,
        "DELETE",
        "/ocs/v2.php/apps/notifications/api/v2/notifications/42",
    )
    .await;
    mount_call(&server, "DELETE", NOTIFICATIONS).await;

    let nc = client(&server);
    nc.notifications().dismiss(42).await.unwrap();
    nc.notifications().dismiss_all().await.unwrap();
}

#[tokio::test]
async fn notifications_app_missing() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(NOTIFICATIONS))
        .respond_with(ResponseTemplate::new(200).set_body_string(ocs_failure(404, "Not found")))
        .mount(&server)
        .await;

    let err = client(&server).notifications().list().await.unwrap_err();
    assert!(err.is_not_found());
}

#[tokio::test]
async fn create_share_form() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ocs/v2.php/apps/files_sharing/api/v1/shares"))
        .and(header("OCS-APIRequest", "true"))
        .and(body_string_contains("shareType=3"))
        .and(body_string_contains("path=%2Freport.pdf"))
        .respond_with(ocs_body(serde_json::json!({
            "id": "17",
            "share_type": 3,
            "permissions": 1,
            "token": "abc123",
            "url": "https://cloud.example.com/s/abc123"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let share = client(&server)
        .shares()
        .create(
            NewShare::public_link("/report.pdf")
                .permissions(SharePermissions::READ)
                .password("hunter2"),
        )
        .await
        .unwrap();

    assert_eq!(share.id, "17");
    assert_eq!(share.share_type, ShareType::PublicLink);
    assert!(share.is_public_link());
    assert_eq!(share.token.as_deref(), Some("abc123"));
}
