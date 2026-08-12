//! The OCS envelope shared by every non-WebDAV endpoint.
//!
//! Responses wrap the payload as `{"ocs": {"meta": {...}, "data": ...}}`. The
//! outcome is `ocs.meta.statuscode`, not the HTTP status: v1 endpoints answer
//! HTTP 200 for failures too. Success is `100` on v1 and `200` on v2.
//!
//! `OCS-APIRequest: true` is mandatory; without it the server serves an HTML
//! login page. v1 defaults to XML and v2 to JSON, so every call here sends
//! `Accept: application/json` and `?format=json`.

use reqwest::Method;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::client::Nextcloud;
use crate::error::{Error, Result};

pub mod status {
    pub const OK_V1: i32 = 100;
    pub const OK_V2: i32 = 200;
    pub const CREATED: i32 = 201;
    pub const BAD_REQUEST: i32 = 400;
    pub const UNAUTHORISED: i32 = 401;
    pub const FORBIDDEN: i32 = 403;
    pub const NOT_FOUND: i32 = 404;
    pub const FAILURE: i32 = 996;
    pub const NOT_AUTHORISED: i32 = 997;
    pub const LEGACY_NOT_FOUND: i32 = 998;
}

#[derive(Clone, Debug, Deserialize)]
pub struct OcsMeta {
    #[serde(default)]
    pub status: String,
    /// The authoritative result code. See [`status`].
    #[serde(default)]
    pub statuscode: i32,
    #[serde(default)]
    pub message: Option<String>,
    /// Result count for paginated endpoints, sent as a number or `""`.
    #[serde(default)]
    pub totalitems: Option<Value>,
    /// Page size for paginated endpoints, sent as a number or `""`.
    #[serde(default)]
    pub itemsperpage: Option<Value>,
}

impl OcsMeta {
    pub fn is_success(&self) -> bool {
        matches!(
            self.statuscode,
            status::OK_V1 | status::OK_V2 | status::CREATED
        )
    }

    /// `totalitems` as a number, or `None` when it arrives empty.
    pub fn total_items(&self) -> Option<u64> {
        coerce_u64(self.totalitems.as_ref()?)
    }

    /// `itemsperpage` as a number, or `None` when it arrives empty.
    pub fn items_per_page(&self) -> Option<u64> {
        coerce_u64(self.itemsperpage.as_ref()?)
    }
}

fn coerce_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) if !s.is_empty() => s.parse().ok(),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct Envelope {
    ocs: EnvelopeBody,
}

#[derive(Debug, Deserialize)]
struct EnvelopeBody {
    meta: OcsMeta,
    #[serde(default)]
    data: Value,
}

#[derive(Clone, Debug)]
pub struct OcsResponse {
    pub meta: OcsMeta,
    pub data: Value,
}

impl OcsResponse {
    /// Deserialise `data` into `T`, treating an empty array as an empty object.
    pub fn parse<T: DeserializeOwned>(self) -> Result<T> {
        let data = normalise_empty(self.data);
        Ok(serde_json::from_value(data)?)
    }
}

/// PHP serialises an empty map as `[]`, which no struct target accepts.
fn normalise_empty(v: Value) -> Value {
    match v {
        Value::Array(a) if a.is_empty() => Value::Object(serde_json::Map::new()),
        other => other,
    }
}

impl Nextcloud {
    /// Call `path` (server-relative, e.g. `ocs/v2.php/cloud/capabilities`) and
    /// return the raw `data` payload.
    pub(crate) async fn ocs_call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
    ) -> Result<Value> {
        Ok(self.ocs_call_full(method, path, query, form).await?.data)
    }

    /// Perform an OCS call and return both the metadata and the payload.
    ///
    /// Use this over [`ocs_call`](Self::ocs_call) when the endpoint paginates
    /// and you need `totalitems`.
    pub(crate) async fn ocs_call_full(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
    ) -> Result<OcsResponse> {
        let url = self.url(path)?;

        let mut rb = self
            .request(method.clone(), url)
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .query(&[("format", "json")]);

        if !query.is_empty() {
            rb = rb.query(query);
        }
        if !form.is_empty() {
            rb = rb.form(form);
        }

        let resp = rb.send().await?;
        let http_status = resp.status();
        let body = resp.text().await?;

        let envelope: Envelope = serde_json::from_str(&body).map_err(|e| {
            // HTML body: header stripped somewhere, or the path is wrong
            Error::UnexpectedResponse(format!(
                "{method} {path} returned HTTP {http_status} with a body that is not an OCS \
                 envelope ({e}): {}",
                truncate(&body, 300)
            ))
        })?;

        if !envelope.ocs.meta.is_success() {
            return Err(Error::Ocs {
                code: envelope.ocs.meta.statuscode,
                message: envelope
                    .ocs
                    .meta
                    .message
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| format!("{method} {path} failed")),
            });
        }

        Ok(OcsResponse {
            meta: envelope.ocs.meta,
            data: envelope.ocs.data,
        })
    }

    pub(crate) async fn ocs_typed<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
    ) -> Result<T> {
        self.ocs_call_full(method, path, query, form).await?.parse()
    }

    /// Call an endpoint for its success or failure, discarding the payload.
    pub(crate) async fn ocs_unit(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
    ) -> Result<()> {
        self.ocs_call_full(method, path, query, form).await?;
        Ok(())
    }

    /// Call an endpoint this crate does not model, with the envelope and error
    /// mapping still applied.
    ///
    /// ```no_run
    /// # async fn demo(nc: &nextcloud::Nextcloud) -> Result<(), nextcloud::Error> {
    /// let data = nc
    ///     .ocs_raw(
    ///         reqwest::Method::GET,
    ///         "ocs/v2.php/apps/serverinfo/api/v1/info",
    ///         &[],
    ///         &[],
    ///     )
    ///     .await?;
    /// # let _ = data;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ocs_raw(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
    ) -> Result<OcsResponse> {
        self.ocs_call_full(method, path, query, form).await
    }
}

/// Nextcloud expects the literal strings `true`/`false` for boolean form and
/// query values, not `1`/`0`.
pub(crate) fn bool_str(v: bool) -> String {
    if v { "true" } else { "false" }.to_string()
}

/// Accumulator for `application/x-www-form-urlencoded` parameters.
///
/// Unset fields are left out rather than sent empty, which is what the update
/// endpoints use to tell "leave this alone" from "clear this".
#[derive(Default)]
pub(crate) struct Form(Vec<(&'static str, String)>);

impl Form {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set(&mut self, key: &'static str, value: impl Into<String>) {
        self.0.push((key, value.into()));
    }

    pub(crate) fn opt(&mut self, key: &'static str, value: Option<&String>) {
        if let Some(v) = value {
            self.set(key, v.clone());
        }
    }

    pub(crate) fn opt_display(&mut self, key: &'static str, value: Option<impl ToString>) {
        if let Some(v) = value {
            self.set(key, v.to_string());
        }
    }

    pub(crate) fn opt_bool(&mut self, key: &'static str, value: Option<bool>) {
        if let Some(v) = value {
            self.set(key, bool_str(v));
        }
    }

    pub(crate) fn finish(self) -> Vec<(&'static str, String)> {
        self.0
    }
}

/// Build the `search`/`limit`/`offset` query the OCS listing endpoints share.
///
/// Unset parameters are omitted entirely: sending an empty `search` is not the
/// same as not searching.
pub(crate) fn paged_query(
    search: Option<&str>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(s) = search {
        query.push(("search", s.to_string()));
    }
    if let Some(l) = limit {
        query.push(("limit", l.to_string()));
    }
    if let Some(o) = offset {
        query.push(("offset", o.to_string()));
    }
    query
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_array_to_object() {
        #[derive(Deserialize)]
        struct Empty {}
        let r = OcsResponse {
            meta: OcsMeta {
                status: "ok".into(),
                statuscode: 100,
                message: None,
                totalitems: None,
                itemsperpage: None,
            },
            data: serde_json::json!([]),
        };
        assert!(r.parse::<Empty>().is_ok());
    }

    #[test]
    fn non_empty_array() {
        let r = OcsResponse {
            meta: OcsMeta {
                status: "ok".into(),
                statuscode: 100,
                message: None,
                totalitems: None,
                itemsperpage: None,
            },
            data: serde_json::json!([1, 2, 3]),
        };
        assert_eq!(r.parse::<Vec<i32>>().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn pagination_counters() {
        let meta = OcsMeta {
            status: "ok".into(),
            statuscode: 100,
            message: None,
            totalitems: Some(serde_json::json!("42")),
            itemsperpage: Some(serde_json::json!(25)),
        };
        assert_eq!(meta.total_items(), Some(42));
        assert_eq!(meta.items_per_page(), Some(25));

        let blank = OcsMeta {
            status: "ok".into(),
            statuscode: 100,
            message: None,
            totalitems: Some(serde_json::json!("")),
            itemsperpage: None,
        };
        assert_eq!(blank.total_items(), None);
    }

    #[test]
    fn success_codes() {
        let mk = |code| OcsMeta {
            status: "ok".into(),
            statuscode: code,
            message: None,
            totalitems: None,
            itemsperpage: None,
        };
        assert!(mk(100).is_success());
        assert!(mk(200).is_success());
        assert!(!mk(998).is_success());
    }

    #[test]
    fn truncate_utf8() {
        assert_eq!(truncate("héllo", 2), "h...");
        assert_eq!(truncate("abc", 10), "abc");
    }
}
