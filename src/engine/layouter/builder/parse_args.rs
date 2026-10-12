//! Function-signature DSL for CSS math-function argument parsing.
//!
//! `parse_args!` declares a Rust-like signature as a bare identifier list,
//! generates an argument-parsing closure from it, and immediately invokes
//! it with the ambient bindings named by the signature. The signature
//! lists the slot sequence (see [`munch!`](super::munch) for slot forms)
//! and the body — an expression of type `Option<T>`.
//!
//! The head only names the bindings (`name`, `args`, `text_flow_style`,
//! and any extra captures); their types and the return type are inferred
//! from the ambient call, so call sites carry no type annotations.
//!
//! ```text
//! parse_args! {
//!     (<name>, <args>, <text_flow_style> [, <extra>]* [,])
//!     { <ident>: <Type> [, ...] }          // slot list
//!     => { <body> }                        // expression of type Option<T>
//! }
//! ```
//!
//! Slot types resolve through their [`ArgResolver`] impl (`CalcValue`,
//! `f32`, `RoundingStrategy`, …), so the macro carries no per-type
//! rules. The expansion yields `Result<T, ParseError>`; the call site's
//! `report_parse_error` decides what gets logged.
//!
//! [`ArgResolver`]: super::args::ArgResolver

macro_rules! parse_args {
    // ── fixed or variadic slots, no unification ──
    (
        ($name:ident, $args:ident, $tfs:ident $(, $p:ident)* $(,)?)
        { $($slots:tt)* }
        => $body:tt
    ) => {{
        (|$name, $args, $tfs $(, $p)*| {
            let mut __it = CssGroupCursor::new($args);
            munch!(
                [__it, $name, $tfs]
                [$($slots)*] [plain $body]
            )
        })($name, $args, $tfs $(, $p)*)
    }};
}
