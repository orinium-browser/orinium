//! Capability set inherited down the frame tree.
//!
//! A browsing context never decides its own privileges from its URL alone.
//! Instead its effective [`Permissions`] are the intersection of the base
//! capabilities of its own origin and the effective permissions of its parent
//! frame ([`Permissions::inherit`]). By induction a descendant can never hold a
//! capability its top-level ancestor lacks, so a nested or embedded context
//! cannot escalate beyond the root document.
//!
//! Only browser-created chrome origins hold [`Permissions::chrome`] (which
//! includes arbitrary local-file access); ordinary documents, including bundled
//! `resource:///` pages, only ever get [`Permissions::content_for`].

use url::Url;

/// The capabilities a document is allowed to exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Permissions {
    /// May load network-backed schemes (`http`, `https`) and `data:` URLs.
    pub network: bool,
    /// May load `resource://` content, confined to the bundle directory by
    /// [`crate::platform::io::load_resource`].
    pub resource: bool,
    /// May load arbitrary local files via `file://`. Reserved for chrome.
    pub file: bool,
}

impl Permissions {
    /// No capabilities at all.
    pub const fn none() -> Self {
        Self {
            network: false,
            resource: false,
            file: false,
        }
    }

    /// Base capabilities for a document loaded from `url`.
    ///
    /// Every document may use the network; `resource://` documents additionally
    /// reach bundled resources (confined to the bundle root). Local files are
    /// never reachable from content.
    pub fn content_for(url: &Url) -> Self {
        Self {
            network: true,
            resource: url.scheme() == "resource",
            file: false,
        }
    }

    /// Base capabilities for a document loaded from the string form of a URL.
    ///
    /// Unparsable URLs are treated as network-capable opaque content, matching a
    /// document that cannot be resolved to a bundled or local origin.
    pub fn content_for_str(url: &str) -> Self {
        match Url::parse(url) {
            Ok(url) => Self::content_for(&url),
            Err(_) => Self {
                network: true,
                resource: false,
                file: false,
            },
        }
    }

    /// Full capabilities for browser-created origins (shell, DevTools).
    pub const fn chrome() -> Self {
        Self {
            network: true,
            resource: true,
            file: true,
        }
    }

    /// Derives the permissions a child frame may hold, given its parent's
    /// effective permissions. The result is the intersection of the two, so a
    /// child can never exceed any ancestor.
    pub const fn inherit(self, parent: Self) -> Self {
        Self {
            network: self.network && parent.network,
            resource: self.resource && parent.resource,
            file: self.file && parent.file,
        }
    }

    /// Whether this set is privileged enough to be treated as chrome: it can
    /// read any local file and therefore bypasses ordinary CORS read checks.
    pub const fn is_chrome(self) -> bool {
        self.file
    }

    /// Whether a resource addressed by `url` may be loaded under these
    /// permissions.
    pub fn allows(self, url: &Url) -> bool {
        match url.scheme() {
            "http" | "https" | "data" => self.network,
            "resource" => self.resource,
            "file" => self.file,
            _ => false,
        }
    }

    /// Whether a resource addressed by the string form of a URL may be loaded.
    ///
    /// Unparsable URLs are treated as opaque content and only allowed when the
    /// network capability is held.
    pub fn allows_str(self, url: &str) -> bool {
        match Url::parse(url) {
            Ok(url) => self.allows(&url),
            Err(_) => self.network,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn content_resource_page_gets_no_file_access() {
        let perms = Permissions::content_for(&url("resource:///test/index.html"));
        assert!(perms.network);
        assert!(perms.resource);
        assert!(!perms.file);
        assert!(!perms.allows(&url("file:///etc/passwd")));
        assert!(perms.allows(&url("resource:///test/style.css")));
    }

    #[test]
    fn web_page_reaches_network_only() {
        let perms = Permissions::content_for(&url("https://example.test/"));
        assert!(perms.allows(&url("https://example.test/x.js")));
        assert!(perms.allows(&url("data:text/plain,hi")));
        assert!(!perms.allows(&url("resource:///test/index.html")));
        assert!(!perms.allows(&url("file:///etc/passwd")));
    }

    #[test]
    fn chrome_reaches_files() {
        let chrome = Permissions::chrome();
        assert!(chrome.is_chrome());
        assert!(chrome.allows(&url("file:///etc/passwd")));
        assert!(chrome.allows(&url("resource:///test/index.html")));
    }

    #[test]
    fn inherit_never_grants_more_than_the_parent() {
        // A chrome frame embedding a web frame cannot hand down file access.
        let child =
            Permissions::content_for(&url("https://example.test/")).inherit(Permissions::chrome());
        assert!(!child.file);
        assert!(!child.resource);
        assert!(child.network);

        // A web frame embedding a chrome frame also yields web-only.
        let child =
            Permissions::chrome().inherit(Permissions::content_for(&url("https://example.test/")));
        assert!(!child.file);
    }

    #[test]
    fn inherit_of_chrome_and_chrome_stays_chrome() {
        assert_eq!(
            Permissions::chrome().inherit(Permissions::chrome()),
            Permissions::chrome()
        );
    }

    #[test]
    fn inherit_is_idempotent() {
        let perms = Permissions::content_for(&url("resource:///a/b.html"));
        assert_eq!(perms.inherit(perms), perms);
    }
}
