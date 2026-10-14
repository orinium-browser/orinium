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
///
/// Scheme capabilities (`network`/`resource`/`file`) govern *what can be
/// loaded*; they never grant the right to read a cross-origin response. That
/// latter authority is separate: only `privileged` (a browser-created chrome
/// context) bypasses CORS. Keeping the two independent means a capability can
/// never be mistaken for a CORS grant, nor vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Permissions {
    /// May load network-backed schemes (`http`, `https`) and `data:` URLs.
    pub network: bool,
    /// May load `resource://` content, confined to the bundle directory by
    /// [`crate::platform::io::load_resource`].
    pub resource: bool,
    /// May load arbitrary local files via `file://`. Reserved for chrome.
    pub file: bool,
    /// Marks a browser-created chrome context (shell, DevTools). This is the
    /// only authority that bypasses ordinary CORS read checks; it is tracked
    /// separately from the scheme capabilities so the two mechanisms stay
    /// independent.
    pub privileged: bool,
}

impl Permissions {
    /// No capabilities at all.
    pub const fn none() -> Self {
        Self {
            network: false,
            resource: false,
            file: false,
            privileged: false,
        }
    }

    /// Base capabilities for a document loaded from `url`.
    ///
    /// Every document may use the network; `resource://` documents additionally
    /// reach bundled resources (confined to the bundle root). Local files are
    /// never reachable from content, and ordinary documents are never
    /// privileged.
    pub fn content_for(url: &Url) -> Self {
        Self {
            network: true,
            resource: url.scheme() == "resource",
            file: false,
            privileged: false,
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
                privileged: false,
            },
        }
    }

    /// Full capabilities for browser-created origins (shell, DevTools).
    pub const fn chrome() -> Self {
        Self {
            network: true,
            resource: true,
            file: true,
            privileged: true,
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
            privileged: self.privileged && parent.privileged,
        }
    }

    /// Effective capabilities for a child frame loaded from `child_url` under
    /// this (parent) set: the base capabilities of the child's own origin,
    /// intersected with the parent's. A child can never gain a capability, and
    /// privilege is never inherited.
    pub fn for_child_frame(self, child_url: &str) -> Self {
        Self::content_for_str(child_url).inherit(self)
    }

    /// Whether this set is a browser-created chrome context: the only authority
    /// that bypasses ordinary CORS read checks.
    pub const fn is_chrome(self) -> bool {
        self.privileged
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
        assert!(chrome.privileged);
        assert!(chrome.allows(&url("file:///etc/passwd")));
        assert!(chrome.allows(&url("resource:///test/index.html")));
    }

    #[test]
    fn privilege_is_separate_from_scheme_capabilities() {
        // Every scheme capability can be present without any CORS-bypass
        // authority: privilege is not implied by reaching files or resources.
        let permissive_content = Permissions {
            network: true,
            resource: true,
            file: true,
            privileged: false,
        };
        assert!(!permissive_content.is_chrome());
        assert!(permissive_content.allows(&url("file:///etc/passwd")));
    }

    #[test]
    fn inherit_never_grants_more_than_the_parent() {
        // A chrome frame embedding a web frame cannot hand down file access or
        // privilege.
        let child =
            Permissions::content_for(&url("https://example.test/")).inherit(Permissions::chrome());
        assert!(!child.file);
        assert!(!child.resource);
        assert!(!child.privileged);
        assert!(child.network);

        // A web frame embedding a chrome frame also yields web-only.
        let child =
            Permissions::chrome().inherit(Permissions::content_for(&url("https://example.test/")));
        assert!(!child.file);
        assert!(!child.privileged);
    }

    #[test]
    fn privilege_never_reappears_down_the_tree() {
        // chrome -> web -> chrome: once privilege is lost it cannot return,
        // even if the deepest frame is itself a chrome URL.
        let web = Permissions::chrome().for_child_frame("https://example.test/");
        let regressed = web.for_child_frame("resource:///shell.html");
        assert!(!regressed.privileged);
        assert!(!regressed.is_chrome());
    }

    #[test]
    fn for_child_frame_uses_the_child_origin_not_the_parent() {
        // A resource child of a chrome parent keeps bundle access but is not
        // privileged; a web child of a chrome parent loses resource access too.
        let chrome = Permissions::chrome();
        let resource_child = chrome.for_child_frame("resource:///test/index.html");
        assert!(resource_child.resource);
        assert!(!resource_child.file);
        assert!(!resource_child.privileged);

        let web_child = chrome.for_child_frame("https://example.test/");
        assert!(!web_child.resource);
        assert!(!web_child.file);
        assert!(!web_child.privileged);

        // An unparsable child URL is treated as opaque network content.
        let opaque_child = chrome.for_child_frame("not a url");
        assert!(opaque_child.network);
        assert!(!opaque_child.resource);
        assert!(!opaque_child.file);
        assert!(!opaque_child.privileged);
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
