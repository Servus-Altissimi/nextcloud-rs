//! WebDAV wiring: uploads, mutations, trash, and version history.

use wiremock::matchers::{body_string_contains, header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

mod common;
use common::{client, dav_response, mount_propfind, mount_status, multistatus};
use nextcloud::files::{ArchiveFormat, DEFAULT_CHUNK_SIZE, Depth, MIN_CHUNK_SIZE};
use nextcloud::{Error, Nextcloud};

const ROOT: &str = "/remote.php/dav/files/alice";

#[tokio::test]
async fn delete() {
    let server = MockServer::start().await;
    mount_status(&server, "DELETE", "/remote.php/dav/files/alice/a.txt", 204).await;

    client(&server).files().delete("/a.txt").await.unwrap();
}

#[tokio::test]
async fn move_headers() {
    let server = MockServer::start().await;

    Mock::given(method("MOVE"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .and(header(
            "Destination",
            format!("{}{ROOT}/b.txt", server.uri()).as_str(),
        ))
        .and(header("Overwrite", "T"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .files()
        .move_to("/a.txt", "/b.txt", true)
        .await
        .unwrap();
}

#[tokio::test]
async fn copy_no_overwrite() {
    let server = MockServer::start().await;

    Mock::given(method("COPY"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .and(header("Overwrite", "F"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .files()
        .copy_to("/a.txt", "/b.txt", false)
        .await
        .unwrap();
}

#[tokio::test]
async fn move_conflict() {
    let server = MockServer::start().await;

    // 412 is the answer when Overwrite: F meets an existing destination
    Mock::given(method("MOVE"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .respond_with(ResponseTemplate::new(412).set_body_string("exists"))
        .mount(&server)
        .await;

    let err = client(&server)
        .files()
        .move_to("/a.txt", "/b.txt", false)
        .await
        .unwrap_err();

    match err {
        Error::Dav { status, method, .. } => {
            assert_eq!(status, 412);
            assert_eq!(method, "MOVE");
        }
        other => panic!("expected a DAV error, got {other:?}"),
    }
}

#[tokio::test]
async fn create_dir_all_error() {
    let server = MockServer::start().await;

    Mock::given(method("MKCOL"))
        .and(path("/remote.php/dav/files/alice/a"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server)
        .files()
        .create_dir_all("/a/b/c")
        .await
        .unwrap_err();
    assert!(err.is_auth_error(), "expected auth error, got {err:?}");
}

#[tokio::test]
async fn mkcol() {
    let server = MockServer::start().await;
    mount_status(&server, "MKCOL", "/remote.php/dav/files/alice/new", 201).await;

    client(&server).files().create_dir("/new").await.unwrap();
}

#[tokio::test]
async fn set_favourite() {
    let server = MockServer::start().await;

    Mock::given(method("PROPPATCH"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .and(header("Content-Type", "application/xml; charset=utf-8"))
        .and(body_string_contains("<oc:favorite>1</oc:favorite>"))
        .respond_with(ResponseTemplate::new(207).set_body_string(multistatus("")))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .files()
        .set_favorite("/a.txt", true)
        .await
        .unwrap();
}

#[tokio::test]
async fn unset_favourite() {
    let server = MockServer::start().await;

    Mock::given(method("PROPPATCH"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .and(body_string_contains("<oc:favorite>0</oc:favorite>"))
        .respond_with(ResponseTemplate::new(207).set_body_string(multistatus("")))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .files()
        .set_favorite("/a.txt", false)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_favourites() {
    let server = MockServer::start().await;

    let body = multistatus(&format!(
        "{}{}",
        dav_response(
            "/remote.php/dav/files/alice/",
            "<d:resourcetype><d:collection/></d:resourcetype>"
        ),
        dav_response(
            "/remote.php/dav/files/alice/liked.txt",
            "<d:resourcetype/><oc:favorite>1</oc:favorite><oc:fileid>5</oc:fileid>"
        )
    ));

    Mock::given(method("REPORT"))
        .and(path("/remote.php/dav/files/alice"))
        .and(body_string_contains("<oc:favorite>1</oc:favorite>"))
        .and(body_string_contains("oc:filter-rules"))
        .respond_with(ResponseTemplate::new(207).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;

    let favourites = client(&server).files().favorites("/").await.unwrap();
    assert_eq!(favourites.len(), 1);
    assert_eq!(favourites[0].name(), "liked.txt");
}

#[tokio::test]
async fn download() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello".to_vec()))
        .mount(&server)
        .await;

    let body = client(&server).files().download("/a.txt").await.unwrap();
    assert_eq!(&body[..], b"hello");
}

#[tokio::test]
async fn download_folder() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/remote.php/dav/files/alice/Docs"))
        .and(header("Accept", "application/x-tar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"tarball".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let archive = client(&server)
        .files()
        .download_folder("/Docs", ArchiveFormat::Tar)
        .await
        .unwrap();
    assert_eq!(&archive[..], b"tarball");
}

#[tokio::test]
async fn read_range() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/remote.php/dav/files/alice/a.bin"))
        .and(header("Range", "bytes=5-9"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(vec![1u8; 5]))
        .expect(1)
        .mount(&server)
        .await;

    let bytes = client(&server)
        .files()
        .download_range("/a.bin", 5, 9)
        .await
        .unwrap();
    assert_eq!(bytes.len(), 5);
}

#[tokio::test]
async fn read_range_empty_file() {
    let server = MockServer::start().await;

    mount_propfind(
        &server,
        "/remote.php/dav/files/alice/empty.txt",
        multistatus(&dav_response(
            "/remote.php/dav/files/alice/empty.txt",
            "<d:resourcetype/><d:getcontentlength>0</d:getcontentlength>",
        )),
    )
    .await;

    let read = client(&server)
        .files()
        .read_range("/empty.txt", 0, None)
        .await
        .unwrap()
        .expect("an empty file is satisfiable");

    assert!(read.is_empty());
    assert_eq!(read.total, 0);
    assert!(!read.is_whole_file());
}

#[tokio::test]
async fn download_stream() {
    use futures_util::StreamExt;

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"streamed".to_vec()))
        .mount(&server)
        .await;

    let mut stream = client(&server)
        .files()
        .download_stream("/a.txt")
        .await
        .unwrap();

    let mut collected = Vec::new();
    while let Some(chunk) = stream.next().await {
        collected.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(collected, b"streamed");
}

#[tokio::test]
async fn propfind_raw() {
    let server = MockServer::start().await;

    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .and(header("Depth", "0"))
        .and(body_string_contains("<oc:tags/>"))
        .respond_with(
            ResponseTemplate::new(207).set_body_string(multistatus(&dav_response(
                "/remote.php/dav/files/alice/a.txt",
                "<oc:tags>holiday</oc:tags>",
            ))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let responses = client(&server)
        .files()
        .propfind_raw("/a.txt", Depth::Zero, &[("http://owncloud.org/ns", "tags")])
        .await
        .unwrap();

    assert_eq!(
        responses[0].text("http://owncloud.org/ns", "tags"),
        Some("holiday")
    );
}

#[tokio::test]
async fn stat_empty_multistatus() {
    let server = MockServer::start().await;

    mount_propfind(
        &server,
        "/remote.php/dav/files/alice/gone.txt",
        multistatus(""),
    )
    .await;

    let err = client(&server).files().stat("/gone.txt").await.unwrap_err();
    assert!(err.to_string().contains("empty multistatus"));
}

#[tokio::test]
async fn upload_auto_small() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/remote.php/dav/files/alice/small.bin"))
        .respond_with(ResponseTemplate::new(201).insert_header("OC-ETag", "\"e1\""))
        .expect(1)
        .mount(&server)
        .await;

    let result = client(&server)
        .files()
        .upload_auto("/small.bin", vec![0u8; 1024])
        .await
        .unwrap();
    assert_eq!(result.etag.as_deref(), Some("\"e1\""));
}

#[tokio::test]
async fn upload_chunked() {
    let server = MockServer::start().await;
    let destination = format!("{}{ROOT}/big.bin", server.uri());

    Mock::given(method("MKCOL"))
        .and(path_regex(r"^/remote\.php/dav/uploads/alice/[^/]+$"))
        .and(header("Destination", destination.as_str()))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/remote\.php/dav/uploads/alice/[^/]+/000\d\d$",
        ))
        .and(header("OC-Total-Length", "12582912"))
        .respond_with(ResponseTemplate::new(201))
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("MOVE"))
        .and(path_regex(r"^/remote\.php/dav/uploads/alice/[^/]+/\.file$"))
        .and(header("Destination", destination.as_str()))
        .respond_with(ResponseTemplate::new(201).insert_header("OC-FileId", "42"))
        .expect(1)
        .mount(&server)
        .await;

    let data = vec![7u8; 12 * 1024 * 1024];
    let result = client(&server)
        .files()
        .upload_large("/big.bin", &data, DEFAULT_CHUNK_SIZE as usize)
        .await
        .unwrap();

    assert_eq!(result.file_id.as_deref(), Some("42"));
}

#[tokio::test]
async fn upload_chunk_failure_aborts() {
    let server = MockServer::start().await;

    Mock::given(method("MKCOL"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"^/remote\.php/dav/uploads/"))
        .respond_with(ResponseTemplate::new(507).set_body_string("quota exceeded"))
        .mount(&server)
        .await;

    // the abort is what stops the server sitting on the session for 24h
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/remote\.php/dav/uploads/alice/[^/]+$"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server)
        .files()
        .upload_large(
            "/big.bin",
            vec![0u8; 6 * 1024 * 1024],
            MIN_CHUNK_SIZE as usize,
        )
        .await
        .unwrap_err();

    match err {
        Error::Dav { status, .. } => assert_eq!(status, 507),
        other => panic!("expected the server's 507, got {other:?}"),
    }
}

#[tokio::test]
async fn chunk_index_out_of_range() {
    let server = MockServer::start().await;

    Mock::given(method("MKCOL"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let nc = client(&server);
    let upload = nc.files().chunked_upload("/big.bin", None).await.unwrap();

    let err = upload.put_chunk_at(0, b"x".to_vec()).await.unwrap_err();
    assert!(err.to_string().contains("outside the permitted range"));
    assert_eq!(upload.chunks_sent(), 0);
    assert!(!upload.upload_id().is_empty());
}

#[tokio::test]
async fn upload_chunked_mtime() {
    let server = MockServer::start().await;

    Mock::given(method("MKCOL"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(method("MOVE"))
        .and(header("X-OC-MTime", "1700000000"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    let nc = client(&server);
    let mut upload = nc.files().chunked_upload("/big.bin", None).await.unwrap();
    upload.set_mtime(1700000000);
    upload.finish().await.unwrap();
}

#[tokio::test]
async fn trash_list() {
    let server = MockServer::start().await;

    let body = multistatus(&format!(
        "{}{}",
        dav_response(
            "/remote.php/dav/trashbin/alice/trash/",
            "<d:resourcetype><d:collection/></d:resourcetype>"
        ),
        dav_response(
            "/remote.php/dav/trashbin/alice/trash/notes.txt.d1700000000",
            "<d:resourcetype/><d:getcontentlength>9</d:getcontentlength>\
             <nc:trashbin-filename>notes.txt</nc:trashbin-filename>\
             <nc:trashbin-original-location>Documents/notes.txt</nc:trashbin-original-location>\
             <nc:trashbin-deletion-time>1700000000</nc:trashbin-deletion-time>"
        )
    ));

    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/trashbin/alice/trash"))
        .and(header("Depth", "1"))
        .respond_with(ResponseTemplate::new(207).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;

    let entries = client(&server).files().trashbin().list().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_name(), "notes.txt");
    assert_eq!(
        entries[0].original_location.as_deref(),
        Some("Documents/notes.txt")
    );
}

#[tokio::test]
async fn trash_restore() {
    let server = MockServer::start().await;

    let listing = multistatus(&dav_response(
        "/remote.php/dav/trashbin/alice/trash/notes.txt.d1700000000",
        "<d:resourcetype/>",
    ));
    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/trashbin/alice/trash"))
        .respond_with(ResponseTemplate::new(207).set_body_string(listing))
        .mount(&server)
        .await;

    let restore_target = format!(
        "{}/remote.php/dav/trashbin/alice/restore/notes.txt.d1700000000",
        server.uri()
    );
    Mock::given(method("MOVE"))
        .and(path(
            "/remote.php/dav/trashbin/alice/trash/notes.txt.d1700000000",
        ))
        .and(header("Destination", restore_target.as_str()))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    let nc = client(&server);
    let entry = nc.files().trashbin().list().await.unwrap().remove(0);
    nc.files().trashbin().restore(&entry).await.unwrap();
}

#[tokio::test]
async fn trash_delete_and_empty() {
    let server = MockServer::start().await;

    let listing = multistatus(&dav_response(
        "/remote.php/dav/trashbin/alice/trash/a.d1",
        "<d:resourcetype/>",
    ));
    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/trashbin/alice/trash"))
        .respond_with(ResponseTemplate::new(207).set_body_string(listing))
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path("/remote.php/dav/trashbin/alice/trash/a.d1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/remote.php/dav/trashbin/alice/trash"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let nc = client(&server);
    let entry = nc.files().trashbin().list().await.unwrap().remove(0);
    nc.files()
        .trashbin()
        .delete_permanently(&entry)
        .await
        .unwrap();
    nc.files().trashbin().empty().await.unwrap();
}

#[tokio::test]
async fn version_list() {
    let server = MockServer::start().await;

    let body = multistatus(&format!(
        "{}{}",
        dav_response(
            "/remote.php/dav/versions/alice/versions/42/",
            "<d:resourcetype><d:collection/></d:resourcetype>"
        ),
        dav_response(
            "/remote.php/dav/versions/alice/versions/42/1700000000",
            "<d:getcontentlength>1024</d:getcontentlength>\
             <nc:version-label>before edit</nc:version-label>"
        )
    ));

    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/versions/alice/versions/42"))
        .respond_with(ResponseTemplate::new(207).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;

    let versions = client(&server).files().versions().list(42).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].label.as_deref(), Some("before edit"));
    assert_eq!(versions[0].timestamp().unwrap().timestamp(), 1700000000);
}

#[tokio::test]
async fn version_download_and_restore() {
    let server = MockServer::start().await;

    let listing = multistatus(&dav_response(
        "/remote.php/dav/versions/alice/versions/42/1700000000",
        "<d:getcontentlength>4</d:getcontentlength>",
    ));
    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/versions/alice/versions/42"))
        .respond_with(ResponseTemplate::new(207).set_body_string(listing))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/remote.php/dav/versions/alice/versions/42/1700000000",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"old!".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let restore_target = format!(
        "{}/remote.php/dav/versions/alice/restore/1700000000",
        server.uri()
    );
    Mock::given(method("MOVE"))
        .and(path(
            "/remote.php/dav/versions/alice/versions/42/1700000000",
        ))
        .and(header("Destination", restore_target.as_str()))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    let nc = client(&server);
    let version = nc.files().versions().list(42).await.unwrap().remove(0);

    let body = nc.files().versions().download(&version).await.unwrap();
    assert_eq!(&body[..], b"old!");

    nc.files().versions().restore(&version).await.unwrap();
}

#[tokio::test]
async fn odd_names_on_the_wire() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path(
            "/remote.php/dav/files/alice/holiday%20photos/track%20%231.mp3",
        ))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .files()
        .upload("/holiday photos/track #1.mp3", b"id3".to_vec())
        .await
        .unwrap();
}

#[tokio::test]
async fn whoami_sets_user_id() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ocs/v2.php/cloud/user"))
        .respond_with(ResponseTemplate::new(200).set_body_string(common::ocs(
            serde_json::json!({"id": "carol", "enabled": true}),
        )))
        .mount(&server)
        .await;

    mount_propfind(
        &server,
        "/remote.php/dav/files/carol",
        multistatus(&dav_response(
            "/remote.php/dav/files/carol/",
            "<d:resourcetype><d:collection/></d:resourcetype>",
        )),
    )
    .await;

    let mut nc = Nextcloud::builder(server.uri())
        .bearer_auth("token")
        .build()
        .unwrap();

    assert!(matches!(
        nc.files().list("/").await.unwrap_err(),
        Error::MissingUserId
    ));

    nc.whoami().await.unwrap();
    assert_eq!(nc.user_id(), Some("carol"));
    nc.files().list("/").await.unwrap();
}

#[tokio::test]
async fn dav_auth_header() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/remote.php/dav/files/alice/a.txt"))
        .and(|req: &Request| {
            req.headers
                .get("authorization")
                .map(|v| v.to_str().unwrap_or_default().starts_with("Basic "))
                .unwrap_or(false)
        })
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).files().download("/a.txt").await.unwrap();
}

#[tokio::test]
async fn list_dir() {
    let server = MockServer::start().await;

    let body = multistatus(&format!(
        "{}{}",
        dav_response(
            "/remote.php/dav/files/alice/Documents/",
            "<d:resourcetype><d:collection/></d:resourcetype><oc:fileid>1</oc:fileid>"
        ),
        dav_response(
            "/remote.php/dav/files/alice/Documents/report.pdf",
            "<d:resourcetype/><d:getcontentlength>2048</d:getcontentlength>\
             <d:getcontenttype>application/pdf</d:getcontenttype>\
             <oc:fileid>2</oc:fileid><oc:favorite>1</oc:favorite>"
        )
    ));

    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/files/alice/Documents"))
        .and(header("Depth", "1"))
        .and(body_string_contains("<oc:fileid/>"))
        .respond_with(ResponseTemplate::new(207).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;

    let entries = client(&server).files().list("/Documents").await.unwrap();

    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.name(), "report.pdf");
    assert_eq!(e.path, "/Documents/report.pdf");
    assert_eq!(e.size, Some(2048));
    assert_eq!(e.file_id, Some(2));
    assert!(e.favorite);
    assert!(!e.is_directory);
}

#[tokio::test]
async fn dav_error_status() {
    let server = MockServer::start().await;

    Mock::given(method("PROPFIND"))
        .and(path("/remote.php/dav/files/alice/nope"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let nc = client(&server);
    let err = nc.files().stat("/nope").await.unwrap_err();
    assert!(err.is_not_found());

    assert!(!nc.files().exists("/nope").await.unwrap());
}

#[tokio::test]
async fn upload_etag() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/remote.php/dav/files/alice/notes.txt"))
        .and(header("X-OC-MTime", "1700000000"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("OC-ETag", "\"deadbeef\"")
                .insert_header("OC-FileId", "99"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let result = client(&server)
        .files()
        .upload_with("/notes.txt", b"hello".to_vec(), Some(1700000000), None)
        .await
        .unwrap();

    assert_eq!(result.etag.as_deref(), Some("\"deadbeef\""));
    assert_eq!(result.file_id.as_deref(), Some("99"));
}

#[tokio::test]
async fn create_dir_all_existing() {
    let server = MockServer::start().await;

    // 405: MKCOL on a collection that already exists
    Mock::given(method("MKCOL"))
        .and(path("/remote.php/dav/files/alice/a"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;

    Mock::given(method("MKCOL"))
        .and(path("/remote.php/dav/files/alice/a/b"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .files()
        .create_dir_all("/a/b")
        .await
        .unwrap();
}

#[tokio::test]
async fn read_range_clamps() {
    let server = MockServer::start().await;

    let props = multistatus(&dav_response(
        "/remote.php/dav/files/alice/film.mp4",
        "<d:resourcetype/><d:getcontentlength>1000</d:getcontentlength>\
         <d:getcontenttype>video/mp4</d:getcontenttype>",
    ));
    mount_propfind(&server, "/remote.php/dav/files/alice/film.mp4", props).await;

    Mock::given(method("GET"))
        .and(path("/remote.php/dav/files/alice/film.mp4"))
        .and(header("Range", "bytes=100-999"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(vec![7u8; 900]))
        .expect(1)
        .mount(&server)
        .await;

    let read = client(&server)
        .files()
        .read_range("/film.mp4", 100, None)
        .await
        .unwrap()
        .expect("range is satisfiable");

    assert_eq!(read.total, 1000);
    assert_eq!(read.start, 100);
    assert_eq!(read.end, 999);
    assert_eq!(read.content_range(), "bytes 100-999/1000");
    assert_eq!(read.content_type.as_deref(), Some("video/mp4"));
    assert_eq!(read.len(), 900);
    assert!(!read.is_whole_file());
}

#[tokio::test]
async fn read_range_past_end() {
    let server = MockServer::start().await;

    let props = multistatus(&dav_response(
        "/remote.php/dav/files/alice/small.bin",
        "<d:resourcetype/><d:getcontentlength>10</d:getcontentlength>",
    ));
    mount_propfind(&server, "/remote.php/dav/files/alice/small.bin", props).await;

    let outcome = client(&server)
        .files()
        .read_range("/small.bin", 10, None)
        .await
        .unwrap();

    assert!(outcome.is_none(), "expected 416 territory");
}
