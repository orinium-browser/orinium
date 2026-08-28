//! Compile-time-gated profiling instrumentation.
//!
//! Timers are declared with [`macro@perf_scope`] and results are reported
//! through [`macro@profile_log`]. Both expand to nothing unless the `profile`
//! feature is enabled.

/// Declares a named timer for the enclosing scope.
///
/// ```ignore
/// fn foo() {
///     perf_scope!(total);
///
///     do_something();
///
///     profile_log!(
///         target: "perf",
///         log::Level::Info,
///         "foo: {:?}",
///         total.elapsed(),
///     );
/// }
/// ```
///
/// Expands to nothing when the `profile` feature is disabled, so every use of
/// the declared binding must live inside a [`macro@profile_log!`] invocation
/// (or its own `#[cfg(any(feature = "profile", debug_assertions))]` gate).
#[macro_export]
macro_rules! perf_scope {
    ($name:ident) => {
        #[cfg(any(feature = "profile", debug_assertions))]
        let $name = std::time::Instant::now();
    };
}

/// Logs profiling information; compiled out unless the `profile` feature is
/// enabled.
///
/// Arguments are neither evaluated nor formatted when disabled.
#[macro_export]
macro_rules! profile_log {
    (target: $target:expr, $level:expr, $($arg:tt)+) => {{
        #[cfg(any(feature = "profile", debug_assertions))]
        log::log!(target: $target, $level, $($arg)+);
    }};
}

/// Shortens `text` to a bounded preview for profile log output.
pub fn text_preview(text: &str) -> String {
    const MAX_LEN: usize = 40;

    if text.len() <= MAX_LEN {
        return text.to_string();
    }
    let mut end = MAX_LEN;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}
