//! Origin model for rendered documents.
//!
//! See <https://html.spec.whatwg.org/multipage/origin.html>. The browser layer
//! uses origins to:
//! - expose a consistent `origin` through the `window` / `location` /
//!   `document` JavaScript APIs,
//! - gate access to internal schemes (`resource:`, ...) from web pages,
//! - compute `Origin` / `Referer` request headers without leaking bundled
//!   resource URLs to external servers,
//! - enforce same-origin / cross-origin visibility for `fetch()` and
//!   `XMLHttpRequest`.

use url::{Origin as UrlOrigin, Url};

/// The origin of a document or a fetch initiator.
///
/// Network schemes (`http`, `https`, `ftp`, `ws`, `wss`) become tuple origins
/// whose equality ignores default ports. Everything else (`resource:`, `data:`,
/// `about:`, `file:`, custom schemes) is opaque: it serializes as `"null"` in
/// JavaScript and never compares equal to another origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin(UrlOrigin);

impl Origin {
    /// Computes the origin of a parsed URL.
    pub fn from_url(url: &Url) -> Self {
        Self(url.origin())
    }

    /// Computes the origin of a URL string, falling back to a fresh opaque
    /// origin when the string does not parse.
    pub fn from_url_string(url: &str) -> Self {
        match Url::parse(url) {
            Ok(url) => Self::from_url(&url),
            Err(_) => Self::opaque(),
        }
    }

    /// A fresh opaque origin (internal schemes, parse failures, ...).
    pub fn opaque() -> Self {
        Self(UrlOrigin::new_opaque())
    }

    /// Serialization exposed to JavaScript: `scheme://host[:port]` or `"null"`.
    pub fn ascii_serialization(&self) -> String {
        self.0.ascii_serialization()
    }

    /// Whether this is an `http(s)://` tuple origin that proxies a real web page.
    pub fn is_network(&self) -> bool {
        self.0.is_tuple()
    }

    /// Whether this is an opaque origin backed by an internal scheme.
    pub fn is_opaque(&self) -> bool {
        !self.0.is_tuple()
    }

    /// Same-origin check. Opaque origins are never equal to any other origin.
    pub fn same_origin(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn network_origins_equal_with_default_port_normalization() {
        let a = Origin::from_url(&url("https://example.test/path"));
        let b = Origin::from_url(&url("https://example.test:443/other"));
        assert_eq!(a, b);
        assert!(a.same_origin(&b));
        assert_eq!(a.ascii_serialization(), "https://example.test");
    }

    #[test]
    fn non_default_ports_make_distinct_origins() {
        let a = Origin::from_url(&url("https://example.test:443/"));
        let b = Origin::from_url(&url("https://example.test:8443/"));
        assert!(a.is_network());
        assert!(b.is_network());
        assert!(!a.same_origin(&b));
        assert_eq!(b.ascii_serialization(), "https://example.test:8443");
    }

    #[test]
    fn scheme_and_host_differences_break_origin_equality() {
        let https = Origin::from_url(&url("https://example.test/"));
        let http = Origin::from_url(&url("http://example.test/"));
        let other_host = Origin::from_url(&url("https://other.test/"));
        assert!(!https.same_origin(&http));
        assert!(!https.same_origin(&other_host));
    }

    #[test]
    fn internal_schemes_are_opaque_and_serialize_as_null() {
        for raw in [
            "resource:///devtools/index.html",
            "data:text/plain,hello",
            "about:blank",
        ] {
            let origin = Origin::from_url(&url(raw));
            assert!(origin.is_opaque(), "{} should be opaque", raw);
            assert_eq!(origin.ascii_serialization(), "null");
        }
    }

    #[test]
    fn opaque_origins_never_compare_equal() {
        let a = Origin::from_url_string("resource:///devtools/index.html");
        let b = Origin::from_url_string("resource:///devtools/index.html");
        assert!(!a.same_origin(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn unparsable_url_strings_fall_back_to_opaque() {
        let origin = Origin::from_url_string("::not a url::");
        assert!(origin.is_opaque());
        assert_eq!(origin.ascii_serialization(), "null");
    }
}
