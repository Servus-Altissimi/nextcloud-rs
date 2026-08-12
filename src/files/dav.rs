//! WebDAV plumbing: namespaces, request bodies, and a `multistatus` parser.
//!
//! A response mixes `DAV:` (RFC 4918), `http://owncloud.org/ns` and
//! `http://nextcloud.org/ns`. Prefixes are conventional, not guaranteed, so the
//! parser resolves namespaces instead of matching on `oc:` text.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use quick_xml::NsReader;
use quick_xml::events::{BytesRef, Event};
use quick_xml::name::ResolveResult;

use crate::error::{Error, Result};

pub const NS_DAV: &str = "DAV:";
/// The ownCloud extension namespace, source of most `oc:` properties.
pub const NS_OC: &str = "http://owncloud.org/ns";
pub const NS_NC: &str = "http://nextcloud.org/ns";
pub const NS_OCS: &str = "http://open-collaboration-services.org/ns";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Depth {
    Zero,
    /// The resource and its immediate children. The usual choice for listings.
    #[default]
    One,
    Infinity,
}

impl Depth {
    pub(crate) fn header(self) -> &'static str {
        match self {
            Depth::Zero => "0",
            Depth::One => "1",
            Depth::Infinity => "infinity",
        }
    }
}

/// One property's value: its text, and any elements nested inside it.
///
/// Nested elements land in `children`: `d:resourcetype` holds `collection` for
/// directories, `oc:share-types` one `share-type` per active share.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropValue {
    pub text: String,
    pub children: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct DavResponse {
    /// The raw, still percent-encoded `<d:href>`.
    pub href: String,

    /// Properties that came back in a 2xx `<d:propstat>`, keyed by
    /// `{namespace}local-name`.
    pub props: HashMap<String, PropValue>,
}

pub fn prop_key(ns: &str, name: &str) -> String {
    format!("{{{ns}}}{name}")
}

impl DavResponse {
    pub fn prop(&self, ns: &str, name: &str) -> Option<&PropValue> {
        self.props.get(&prop_key(ns, name))
    }

    /// A property's text, treating an empty element as absent.
    pub fn text(&self, ns: &str, name: &str) -> Option<&str> {
        self.prop(ns, name)
            .map(|p| p.text.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn parsed<T: std::str::FromStr>(&self, ns: &str, name: &str) -> Option<T> {
        self.text(ns, name)?.parse().ok()
    }

    /// Whether `d:resourcetype` marks this as a collection (directory).
    pub fn is_collection(&self) -> bool {
        self.prop(NS_DAV, "resourcetype")
            .map(|p| p.children.iter().any(|(n, _)| n == "collection"))
            .unwrap_or(false)
    }
}

/// Parse a `207 Multi-Status` body, dropping properties from non-2xx
/// `<d:propstat>` blocks.
pub fn parse_multistatus(xml: &[u8]) -> Result<Vec<DavResponse>> {
    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut parser = MultistatusParser::default();
    let mut buf = Vec::new();
    let mut done = false;

    while !done {
        {
            let (resolved, event) = reader
                .read_resolved_event_into(&mut buf)
                .map_err(|e| Error::Xml(e.to_string()))?;

            let ns = match resolved {
                ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
                _ => String::new(),
            };

            match event {
                Event::Eof => done = true,
                Event::Start(ref e) => {
                    let local = local_name(e.local_name().as_ref());
                    parser.start(&ns, &local);
                }
                Event::Empty(ref e) => {
                    let local = local_name(e.local_name().as_ref());
                    parser.start(&ns, &local);
                    parser.end(&local);
                }
                Event::End(ref e) => {
                    let local = local_name(e.local_name().as_ref());
                    parser.end(&local);
                }
                Event::Text(ref t) => {
                    let s = t.xml10_content().map_err(|e| Error::Xml(e.to_string()))?;
                    parser.push_text(&s);
                }
                Event::GeneralRef(ref r) => {
                    let s = resolve_reference(r)?;
                    parser.push_text(&s);
                }
                Event::CData(ref t) => {
                    let s = String::from_utf8_lossy(t.as_ref()).into_owned();
                    parser.push_text(&s);
                }
                _ => {}
            }
        }
        buf.clear();
    }

    Ok(parser.finish())
}

fn local_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// quick-xml reports `&...;` as its own event rather than folding it into the
/// surrounding text, so character references and the five predefined entities
/// are resolved here. Anything else is passed through verbatim.
fn resolve_reference(r: &BytesRef<'_>) -> Result<String> {
    if let Some(c) = r
        .resolve_char_ref()
        .map_err(|e| Error::Xml(e.to_string()))?
    {
        return Ok(c.to_string());
    }
    let name = r.decode().map_err(|e| Error::Xml(e.to_string()))?;
    Ok(match name.as_ref() {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        other => format!("&{other};"),
    })
}

/// Depth-indexed state machine over `multistatus`. The document shape is fixed,
/// so depth alone identifies a node: 2 is `response`, 3 `href` or `propstat`,
/// 4 `prop` or `status`, 5 a property, 6 a nested element.
#[derive(Default)]
struct MultistatusParser {
    depth: usize,
    out: Vec<DavResponse>,
    current: Option<DavResponse>,
    propstat_props: HashMap<String, PropValue>,
    propstat_status: String,
    current_prop: Option<(String, PropValue)>,
    current_child: Option<String>,
    text: String,
}

impl MultistatusParser {
    fn start(&mut self, ns: &str, local: &str) {
        self.text.clear();
        self.depth += 1;

        match self.depth {
            2 if local == "response" => self.current = Some(DavResponse::default()),
            3 if local == "propstat" => {
                self.propstat_props.clear();
                self.propstat_status.clear();
            }
            5 => self.current_prop = Some((prop_key(ns, local), PropValue::default())),
            6 => self.current_child = Some(local.to_string()),
            _ => {}
        }
    }

    fn end(&mut self, local: &str) {
        match self.depth {
            6 => {
                if let Some(name) = self.current_child.take() {
                    let text = self.text.trim().to_string();
                    if let Some((_, prop)) = self.current_prop.as_mut() {
                        prop.children.push((name, text));
                    }
                }
            }
            5 => {
                if let Some((key, mut prop)) = self.current_prop.take() {
                    prop.text = self.text.trim().to_string();
                    self.propstat_props.insert(key, prop);
                }
            }
            4 if local == "status" => self.propstat_status = self.text.trim().to_string(),
            3 if local == "href" => {
                if let Some(cur) = self.current.as_mut() {
                    cur.href = self.text.trim().to_string();
                }
            }
            3 if local == "propstat" => {
                if propstat_succeeded(&self.propstat_status)
                    && let Some(cur) = self.current.as_mut()
                {
                    for (k, v) in self.propstat_props.drain() {
                        cur.props.insert(k, v);
                    }
                }
                self.propstat_props.clear();
            }
            2 if local == "response" => {
                if let Some(cur) = self.current.take() {
                    self.out.push(cur);
                }
            }
            _ => {}
        }

        self.depth = self.depth.saturating_sub(1);
        self.text.clear();
    }

    fn push_text(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn finish(self) -> Vec<DavResponse> {
        self.out
    }
}

/// `<d:status>` reads `HTTP/1.1 200 OK`. Some servers omit it when everything
/// succeeded, so an absent status counts as success.
fn propstat_succeeded(status: &str) -> bool {
    if status.is_empty() {
        return true;
    }
    status
        .split_whitespace()
        .find_map(|tok| tok.parse::<u16>().ok())
        .map(|code| (200..300).contains(&code))
        .unwrap_or(false)
}

const DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

/// Declared on every request body, since one may name properties from all three.
const NAMESPACES: &str =
    r#"xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns""#;

fn prop_elements(props: &[(&str, &str)]) -> String {
    props
        .iter()
        .map(|(ns, name)| format!("<{}:{}/>", prefix_for(ns), name))
        .collect()
}

/// Build a PROPFIND request body requesting `props`, given as
/// `(namespace, local-name)` pairs.
pub fn propfind_body(props: &[(&str, &str)]) -> String {
    format!(
        "{DECLARATION}<d:propfind {NAMESPACES}><d:prop>{}</d:prop></d:propfind>",
        prop_elements(props)
    )
}

pub(crate) fn favorites_body(props: &[(&str, &str)]) -> String {
    format!(
        "{DECLARATION}<oc:filter-files {NAMESPACES}><d:prop>{}</d:prop><oc:filter-rules><oc:favorite>1</oc:favorite></oc:filter-rules></oc:filter-files>",
        prop_elements(props)
    )
}

pub fn proppatch_body(ns: &str, name: &str, value: &str) -> String {
    format!(
        "{DECLARATION}<d:propertyupdate {NAMESPACES}><d:set><d:prop><{p}:{name}>{value}</{p}:{name}></d:prop></d:set></d:propertyupdate>",
        p = prefix_for(ns),
        name = name,
        value = escape_xml(value),
    )
}

/// The last path segment of an href, percent-decoded. Trash and version
/// entries are addressed by it.
pub(crate) fn href_leaf(href: &str) -> String {
    percent_encoding::percent_decode_str(href)
        .decode_utf8_lossy()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The size of a resource: `oc:size` for folders, where it is a recursive
/// total, and `d:getcontentlength` for files.
pub(crate) fn size_of(resp: &DavResponse, is_directory: bool) -> Option<u64> {
    if is_directory {
        resp.parsed::<u64>(NS_OC, "size")
    } else {
        resp.parsed::<u64>(NS_DAV, "getcontentlength")
            .or_else(|| resp.parsed::<u64>(NS_OC, "size"))
    }
}

pub(crate) fn prefix_for(ns: &str) -> &'static str {
    match ns {
        NS_OC => "oc",
        NS_NC => "nc",
        _ => "d",
    }
}

pub(crate) fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Parse an HTTP-date (`Sun, 06 Nov 1994 08:49:37 GMT`), the format
/// `d:getlastmodified` uses.
pub(crate) fn parse_http_date(s: &str) -> Option<DateTime<Utc>> {
    httpdate::parse_http_date(s).ok().map(DateTime::<Utc>::from)
}

pub(crate) fn parse_iso_date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/files/alice/Documents/</d:href>
    <d:propstat>
      <d:prop>
        <d:getlastmodified>Sun, 06 Nov 1994 08:49:37 GMT</d:getlastmodified>
        <d:resourcetype><d:collection/></d:resourcetype>
        <oc:fileid>1234</oc:fileid>
        <oc:size>4096</oc:size>
        <oc:permissions>RGDNVCK</oc:permissions>
        <oc:favorite>0</oc:favorite>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
    <d:propstat>
      <d:prop><nc:has-preview/><d:getcontenttype/></d:prop>
      <d:status>HTTP/1.1 404 Not Found</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/alice/Documents/a%20file.txt</d:href>
    <d:propstat>
      <d:prop>
        <d:getcontentlength>12</d:getcontentlength>
        <d:getcontenttype>text/plain</d:getcontenttype>
        <d:getetag>&quot;abc123&quot;</d:getetag>
        <d:resourcetype/>
        <oc:share-types><oc:share-type>3</oc:share-type><oc:share-type>0</oc:share-type></oc:share-types>
        <nc:has-preview>true</nc:has-preview>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn parses_multistatus() {
        let responses = parse_multistatus(SAMPLE.as_bytes()).unwrap();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].href, "/remote.php/dav/files/alice/Documents/");
    }

    #[test]
    fn resourcetype_collection() {
        let r = parse_multistatus(SAMPLE.as_bytes()).unwrap();
        assert!(r[0].is_collection());
        assert!(!r[1].is_collection());
    }

    #[test]
    fn drops_failed_propstat() {
        let r = parse_multistatus(SAMPLE.as_bytes()).unwrap();
        assert!(r[0].prop(NS_NC, "has-preview").is_none());
        assert!(r[0].prop(NS_DAV, "getcontenttype").is_none());
        assert_eq!(r[0].text(NS_OC, "fileid"), Some("1234"));
    }

    #[test]
    fn ns_resolution() {
        let r = parse_multistatus(SAMPLE.as_bytes()).unwrap();
        assert_eq!(r[0].parsed::<u64>(NS_OC, "size"), Some(4096));
        assert_eq!(r[1].text(NS_NC, "has-preview"), Some("true"));
    }

    #[test]
    fn repeated_children() {
        let r = parse_multistatus(SAMPLE.as_bytes()).unwrap();
        let share_types = r[1].prop(NS_OC, "share-types").unwrap();
        assert_eq!(
            share_types.children,
            vec![
                ("share-type".to_string(), "3".to_string()),
                ("share-type".to_string(), "0".to_string()),
            ]
        );
    }

    #[test]
    fn unescapes_entities() {
        let r = parse_multistatus(SAMPLE.as_bytes()).unwrap();
        assert_eq!(r[1].text(NS_DAV, "getetag"), Some("\"abc123\""));
    }

    #[test]
    fn odd_prefixes() {
        let xml = r#"<?xml version="1.0"?>
<x:multistatus xmlns:x="DAV:" xmlns:oc="http://nextcloud.org/ns">
  <x:response><x:href>/a</x:href><x:propstat><x:prop>
    <oc:has-preview>true</oc:has-preview>
  </x:prop><x:status>HTTP/1.1 200 OK</x:status></x:propstat></x:response>
</x:multistatus>"#;
        let r = parse_multistatus(xml.as_bytes()).unwrap();
        assert_eq!(r[0].text(NS_NC, "has-preview"), Some("true"));
    }

    #[test]
    fn status_line_parsing() {
        assert!(propstat_succeeded("HTTP/1.1 200 OK"));
        assert!(propstat_succeeded("HTTP/1.1 207 Multi-Status"));
        assert!(propstat_succeeded(""));
        assert!(!propstat_succeeded("HTTP/1.1 404 Not Found"));
        assert!(!propstat_succeeded("HTTP/1.1 403 Forbidden"));
    }

    #[test]
    fn propfind_body_xml() {
        let body = propfind_body(&[
            (NS_DAV, "getetag"),
            (NS_OC, "fileid"),
            (NS_NC, "has-preview"),
        ]);
        assert!(body.contains("<d:getetag/>"));
        assert!(body.contains("<oc:fileid/>"));
        assert!(body.contains("<nc:has-preview/>"));
    }

    #[test]
    fn proppatch_escaping() {
        let body = proppatch_body(NS_OC, "favorite", "1");
        assert!(body.contains("<oc:favorite>1</oc:favorite>"));
        assert!(proppatch_body(NS_NC, "x", "a&b").contains("a&amp;b"));
    }

    #[test]
    fn http_dates() {
        let d = parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        assert_eq!(d.timestamp(), 784111777);
        assert!(parse_http_date("nonsense").is_none());
    }

    #[test]
    fn empty_multistatus() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"></d:multistatus>"#;
        assert!(parse_multistatus(xml.as_bytes()).unwrap().is_empty());
    }
}
