//! Function-signature DSL for CSS math-function argument parsing.
//!
//! `parse_args!` declares a Rust-like signature as a bare identifier list,
//! generates an argument-parsing closure from it, and immediately invokes
//! it with the ambient bindings named by the signature. The signature
//! lists the slot sequence (see [`munch!`](super::munch) for slot forms)
//! and the body — either a plain expression or the number/length branches
//! of a unified function.
//!
//! The head only names the bindings (`name`, `args`, `text_flow_style`,
//! and any extra captures); their types and the return type are inferred
//! from the ambient call, so call sites carry no type annotations.
//!
//! ```text
//! // plain form: no operand unification
//! parse_args! {
//!     (<name>, <args>, <text_flow_style> [, <extra>]* [,])
//!     { <ident>: <Type> [, ...] }          // slot list
//!     => { <body> }                        // expression of type Option<T>
//! }
//!
//! // cases form: number/length operand unification (`as` supplies the
//! // binding, listed among the extras, reported by the operand unifier)
//! parse_args! {
//!     (<name>, <args>, <text_flow_style> [, <extra>]* [,])
//!     as <fn_name_ident>
//!     { <ident>: <Type> [, ...] }
//!     number { <body> } length { <body> }
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
    // ── cases form: fixed or variadic slots + number/length operand
    //    unification ──
    (
        ($name:ident, $args:ident, $tfs:ident $(, $p:ident)* $(,)?)
        as $fnid:ident
        { $($slots:tt)* }
        number $num:tt length $len:tt
    ) => {{
        (|$name, $args, $tfs $(, $p)*| {
            let __unify = |values| {
                unify_operands($name, $fnid, values).map(|mode| match mode {
                    OperandMode::Numbers(v) => Ok(v),
                    OperandMode::Lengths(v) => Err(v),
                })
            };
            let mut __it = CssGroupCursor::new($args);
            munch!(
                [()]
                [__it, $name, $tfs, __unify]
                [$($slots)*] [cases $num $len]
            )
        })($name, $args, $tfs $(, $p)*)
    }};
    // ── plain form: fixed or variadic slots, no unification ──
    (
        ($name:ident, $args:ident, $tfs:ident $(, $p:ident)* $(,)?)
        { $($slots:tt)* }
        => $body:tt
    ) => {{
        (|$name, $args, $tfs $(, $p)*| {
            let mut __it = CssGroupCursor::new($args);
            munch!(
                [()]
                [__it, $name, $tfs]
                [$($slots)*] [plain $body]
            )
        })($name, $args, $tfs $(, $p)*)
    }};
}
