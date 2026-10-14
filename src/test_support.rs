//! Helpers shared by unit tests across the crate.
//!
//! Kept intentionally dependency-free so that any layer (`engine`, `platform`,
//! `browser`) can use them without violating the dependency law.

use url::Url;

/// Compile-time assertion that `T` is `Send`.
pub(crate) fn assert_send<T: Send>() {}

/// Compile-time assertion that `T` is `Sync`.
pub(crate) fn assert_sync<T: Sync>() {}

/// Parses a URL for tests, panicking on invalid input.
pub(crate) fn parse_url(s: &str) -> Url {
    Url::parse(s).expect("test URL should parse")
}
