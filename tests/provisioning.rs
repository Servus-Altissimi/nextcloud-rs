//! Provisioning API wiring: users, groups, and apps.

use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{client, mount_call, mount_form, mount_ocs, ocs_body, ocs_failure, ocs_ok};
use nextcloud::{AppFilter, NewUser, UserField};

#[tokio::test]
async fn create_user() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ocs/v1.php/cloud/users"))
        .and(header("OCS-APIRequest", "true"))
        .and(body_string_contains("userid=bob"))
        .and(body_string_contains("displayName=Bob+B"))
        .and(body_string_contains("quota=5+GB"))
        .and(body_string_contains("groups%5B%5D=staff"))
        .and(body_string_contains("subadmin%5B%5D=staff"))
        .respond_with(ocs_ok())
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .users()
        .create(
            NewUser::new("bob")
                .display_name("Bob B")
                .password("pw")
                .quota("5 GB")
                .group("staff")
                .subadmin_of("staff"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn create_user_rejected() {
    let server = MockServer::start().await;

    // 102 = id already taken
    Mock::given(method("POST"))
        .and(path("/ocs/v1.php/cloud/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(ocs_failure(102, "User already exists")),
        )
        .mount(&server)
        .await;

    let err = client(&server)
        .users()
        .create(NewUser::new("bob").password("pw"))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("User already exists"));
    assert!(!err.is_not_found());
}

#[tokio::test]
async fn edit_user() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/ocs/v1.php/cloud/users/bob"))
        .and(body_string_contains("key=email"))
        .and(body_string_contains("value=bob%40example.org"))
        .respond_with(ocs_ok())
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .users()
        .edit("bob", UserField::Email, "bob@example.org")
        .await
        .unwrap();
}

#[tokio::test]
async fn edit_user_raw_field() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/ocs/v1.php/cloud/users/bob"))
        .and(body_string_contains("key=locale"))
        .respond_with(ocs_ok())
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .users()
        .edit("bob", UserField::Other("locale".into()), "en_GB")
        .await
        .unwrap();
}

#[tokio::test]
async fn enable_disable_user() {
    let server = MockServer::start().await;

    for suffix in ["disable", "enable"] {
        Mock::given(method("PUT"))
            .and(path(format!("/ocs/v1.php/cloud/users/bob/{suffix}")))
            .respond_with(ocs_ok())
            .expect(1)
            .mount(&server)
            .await;
    }

    let nc = client(&server);
    nc.users().disable("bob").await.unwrap();
    nc.users().enable("bob").await.unwrap();
}

#[tokio::test]
async fn delete_user() {
    let server = MockServer::start().await;

    mount_call(&server, "DELETE", "/ocs/v1.php/cloud/users/bob").await;
    client(&server).users().delete("bob").await.unwrap();
}

#[tokio::test]
async fn group_membership() {
    let server = MockServer::start().await;

    mount_form(
        &server,
        "POST",
        "/ocs/v1.php/cloud/users/bob/groups",
        "groupid=staff",
    )
    .await;
    mount_form(
        &server,
        "DELETE",
        "/ocs/v1.php/cloud/users/bob/groups",
        "groupid=staff",
    )
    .await;

    let nc = client(&server);
    nc.users().add_to_group("bob", "staff").await.unwrap();
    nc.users().remove_from_group("bob", "staff").await.unwrap();
}

#[tokio::test]
async fn subadmin() {
    let server = MockServer::start().await;

    mount_form(
        &server,
        "POST",
        "/ocs/v1.php/cloud/users/bob/subadmins",
        "groupid=staff",
    )
    .await;
    mount_form(
        &server,
        "DELETE",
        "/ocs/v1.php/cloud/users/bob/subadmins",
        "groupid=staff",
    )
    .await;

    let nc = client(&server);
    nc.users().promote_subadmin("bob", "staff").await.unwrap();
    nc.users().demote_subadmin("bob", "staff").await.unwrap();
}

#[tokio::test]
async fn user_groups() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/users/bob/groups",
        serde_json::json!({"groups": ["staff", "editors"]}),
    )
    .await;
    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/users/bob/subadmins",
        serde_json::json!(["staff"]),
    )
    .await;

    let nc = client(&server);
    assert_eq!(
        nc.users().groups("bob").await.unwrap(),
        vec!["staff", "editors"]
    );
    assert_eq!(
        nc.users().subadmin_groups("bob").await.unwrap(),
        vec!["staff"]
    );
}

#[tokio::test]
async fn get_user_fills_id() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/users/bob",
        serde_json::json!({
            "displayname": "Bob B",
            "email": "bob@example.org",
            "enabled": true,
            "quota": {"free": 100, "used": 50, "quota": -3}
        }),
    )
    .await;

    let user = client(&server).users().get("bob").await.unwrap();
    assert_eq!(user.id, "bob");
    assert_eq!(user.display_name_or_id(), "Bob B");
    assert!(user.quota.is_unlimited());
}

#[tokio::test]
async fn list_users_paging() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v1.php/cloud/users"))
        .and(query_param("search", "al"))
        .and(query_param("limit", "10"))
        .and(query_param("offset", "20"))
        .respond_with(ocs_body(serde_json::json!({"users": ["alice"]})))
        .expect(1)
        .mount(&server)
        .await;

    let users = client(&server)
        .users()
        .list(Some("al"), Some(10), Some(20))
        .await
        .unwrap();
    assert_eq!(users, vec!["alice"]);
}

#[tokio::test]
async fn welcome_mail() {
    let server = MockServer::start().await;

    mount_call(&server, "POST", "/ocs/v1.php/cloud/users/bob/welcome").await;
    client(&server)
        .users()
        .resend_welcome_email("bob")
        .await
        .unwrap();
}

#[tokio::test]
async fn editable_fields() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/user/fields",
        serde_json::json!(["displayname", "email"]),
    )
    .await;

    assert_eq!(
        client(&server).users().editable_fields().await.unwrap(),
        vec!["displayname", "email"]
    );
}

#[tokio::test]
async fn create_group() {
    let server = MockServer::start().await;

    mount_form(&server, "POST", "/ocs/v1.php/cloud/groups", "groupid=staff").await;
    client(&server).groups().create("staff").await.unwrap();
}

#[tokio::test]
async fn rename_group() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/ocs/v1.php/cloud/groups/staff"))
        .and(body_string_contains("key=displayname"))
        .and(body_string_contains("value=The+Staff"))
        .respond_with(ocs_ok())
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .groups()
        .set_display_name("staff", "The Staff")
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_group() {
    let server = MockServer::start().await;

    mount_call(&server, "DELETE", "/ocs/v1.php/cloud/groups/staff").await;
    client(&server).groups().delete("staff").await.unwrap();
}

#[tokio::test]
async fn list_groups() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/groups",
        serde_json::json!({"groups": ["admin", "staff"]}),
    )
    .await;
    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/groups/staff",
        serde_json::json!({"users": ["alice", "bob"]}),
    )
    .await;
    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/groups/staff/subadmins",
        serde_json::json!(["alice"]),
    )
    .await;

    let nc = client(&server);
    assert_eq!(
        nc.groups().list(None, None, None).await.unwrap(),
        vec!["admin", "staff"]
    );
    assert_eq!(
        nc.groups().members("staff").await.unwrap(),
        vec!["alice", "bob"]
    );
    assert_eq!(nc.groups().subadmins("staff").await.unwrap(), vec!["alice"]);
}

#[tokio::test]
async fn search_groups() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v1.php/cloud/groups"))
        .and(query_param("search", "sta"))
        .respond_with(ocs_body(serde_json::json!({"groups": ["staff"]})))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(
        client(&server)
            .groups()
            .list(Some("sta"), None, None)
            .await
            .unwrap(),
        vec!["staff"]
    );
}

#[tokio::test]
async fn list_apps() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v1.php/cloud/apps"))
        .and(query_param("filter", "enabled"))
        .respond_with(ocs_body(serde_json::json!({"apps": ["files", "music"]})))
        .mount(&server)
        .await;

    let apps = client(&server)
        .apps()
        .list(Some(AppFilter::Enabled))
        .await
        .unwrap();
    assert_eq!(apps, vec!["files", "music"]);
}

#[tokio::test]
async fn app_is_enabled() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/apps",
        serde_json::json!({"apps": ["files"]}),
    )
    .await;

    let nc = client(&server);
    assert!(nc.apps().is_enabled("files").await.unwrap());
    assert!(!nc.apps().is_enabled("spreed").await.unwrap());
}

#[tokio::test]
async fn app_info_extra_keys() {
    let server = MockServer::start().await;

    mount_ocs(
        &server,
        "GET",
        "/ocs/v1.php/cloud/apps/music",
        serde_json::json!({
            "id": "music",
            "name": "Music",
            "version": "1.0.0",
            "author": ["Someone"]
        }),
    )
    .await;

    let info = client(&server).apps().info("music").await.unwrap();
    assert_eq!(info.name.as_deref(), Some("Music"));
    assert!(info.extra.contains_key("author"));
}

#[tokio::test]
async fn enable_disable_app() {
    let server = MockServer::start().await;

    for verb in ["POST", "DELETE"] {
        Mock::given(method(verb))
            .and(path("/ocs/v1.php/cloud/apps/music"))
            .respond_with(ocs_ok())
            .expect(1)
            .mount(&server)
            .await;
    }

    let nc = client(&server);
    nc.apps().enable("music").await.unwrap();
    nc.apps().disable("music").await.unwrap();
}

#[tokio::test]
async fn forbidden_is_auth_error() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/ocs/v1.php/cloud/users/bob"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(ocs_failure(997, "Not authorised")),
        )
        .mount(&server)
        .await;

    let err = client(&server).users().delete("bob").await.unwrap_err();
    assert!(err.is_auth_error(), "expected auth error, got {err:?}");
}
