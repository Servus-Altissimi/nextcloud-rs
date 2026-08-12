# nextcloud-rs

Async [Nextcloud](https://docs.nextcloud.com/server/latest/developer_manual/) server API client for Rust.

Covers WebDAV files, the OCS APIs (sharing, provisioning, capabilities, notifications, user status), the `index.php` image routes, and Login Flow v2. Async-only, built on `reqwest` + `tokio`. The library import name is `nextcloud`.

## Roadmap

- [x] WebDAV files, trash bin, version history
- [x] Chunked upload
- [x] OCS sharing
- [x] Provisioning: users, groups, apps
- [x] Capabilities, notifications, user status
- [x] Previews, avatars, public share links
- [x] Login Flow v2
- [ ] Talk (`spreed`): rooms, messages, calls
- [ ] Activity
- [ ] Deck
- [ ] Notes
- [ ] systemtags and comments
- [ ] CalDAV and CardDAV

## Features

- WebDAV files: browse, stat, upload, download, move, copy, delete, favourites.
- Chunked upload for large files. `upload_auto` picks between a plain `PUT` and the chunked protocol by size.
- Trash bin and file version history: list, restore, purge.
- `read_range` answers a `Range` request in one call: metadata lookup, clamping, and the `Content-Range` values a media player needs.
- OCS sharing: create, list, update, delete, mail. Public links, passwords, expiry, permission bitmask.
- Provisioning: users, groups, apps.
- Capabilities, notifications, user status.
- Preview, avatar and public share URL builders. `fetch` sniffs the image type from the bytes and reports a missing preview as `Ok(None)`.
- Login Flow v2, which mints an app password through the browser.
- `ocs_raw` reaches endpoints the crate does not model, with envelope parsing and error mapping applied.
- Unknown values survive: share types, status types and user fields have `Other` variants, `User` and `AppInfo` keep unmodelled JSON keys in an `extra` map.

## Install

```toml
[dependencies]
nextcloud-rs = "0.1"
```

## Quick start

```rust
use nextcloud::Nextcloud;

let nc = Nextcloud::builder("http://127.0.0.1:8080")
    .basic_auth("alice", "app-password")
    .build()?;

for entry in nc.files().list("/Documents").await? {
    println!("{} ({} bytes)", entry.name(), entry.size.unwrap_or(0));
}
```

## Surfaces

| Area | Protocol | Module |
|------|----------|--------|
| Browse, upload, download, move, copy, delete | WebDAV | `files` |
| Chunked upload for large files | WebDAV | `files::upload` |
| Trash bin: list, restore, purge, empty | WebDAV | `files::trashbin` |
| File version history: list, download, restore | WebDAV | `files::versions` |
| Favourites: set, and the `filter-files` REPORT | WebDAV | `files` |
| Shares: create, list, update, delete, mail | OCS | `sharing` |
| Users, groups, apps | OCS (Provisioning) | `provisioning` |
| Server version and capability discovery | OCS | `capabilities` |
| Notifications: list, fetch, dismiss | OCS | `notifications` |
| User status: availability and messages | OCS | `user_status` |
| Thumbnails, avatars, public share links | `index.php` | `preview` |
| Login Flow v2 (app-password handshake) | JSON/HTTP | `auth` |
| Anything not modelled | OCS | `Nextcloud::ocs_raw` |

`files::path` handles remote path arithmetic. `MediaKind` classifies a file as image, video or audio.

## Authentication

Three credential shapes work: basic auth with an app password, basic auth with the account password, and bearer tokens for OIDC-fronted instances. App passwords are revocable, scoped to one client, and unaffected by two-factor authentication.

`LoginFlowV2` mints an app password through the browser:

```rust
let flow = nextcloud::LoginFlowV2::start("http://127.0.0.1:8080").await?;
println!("Authorise at: {}", flow.login_url());

let creds = flow.wait_for_completion().await?;
let nc = nextcloud::Nextcloud::from_login(creds)?;
```

The poll endpoint answers `404` until the user authorises the client, then exactly one `200` carrying the credentials. The poll token lives 20 minutes and the success payload is served once.

Personal WebDAV paths embed the user id. Basic auth adopts the login name automatically; a bearer client gets one from `whoami()` or `NextcloudBuilder::user_id`, and errors with `Error::MissingUserId` until it has one.

`NextcloudBuilder::user_agent` sets the string shown on the grant screen and in the account's connected-devices list.

## wasm

Native and `wasm32-unknown-unknown` both build. On wasm, reqwest compiles down to `fetch`, so the TLS backend, `user_agent` and `timeout` are the browser's business and are compiled out. `LoginFlowV2` polls on a `gloo-timers` sleep and accumulates elapsed time, since `Instant` panics there.

```sh
cargo check --target wasm32-unknown-unknown
```

## Development

```sh
cargo test                                    # unit + integration + doc tests
cargo cov                                     # coverage summary, fails under the gate
cargo cov-html                                # coverage report in a browser
cargo clippy --all-targets -- -D warnings     # also run with --release
cargo fmt --all
```

`cargo cov` needs `cargo install cargo-llvm-cov` and the `llvm-tools-preview`
component. CI runs the same commands on every push and pull request.
