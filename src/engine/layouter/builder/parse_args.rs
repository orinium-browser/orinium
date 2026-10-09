//! Function-signature DSL for CSS math-function argument parsing.
//!
//! `parse_args!` declares a Rust-like signature, generates an
//! argument-parsing function from it, and immediately invokes it with
//! the ambient bindings named by the signature. The signature lists the
//! slot sequence (see [`munch!`](super::munch) for slot forms) and the
//! body — either a plain expression or the number/length branches of a
//! unified function.
//!
//! ```text
//! // plain form: no operand unification
//! parse_args! {
//!     fn <gen>(name: &str, args: &[Vec<CssValue>], text_flow_style: &TextFlowStyle
//!              [, <extra>: <Type>]* [,])
//!     -> <Result<T, ParseError>>
//!     { <ident>: <Type> [, ...] }          // slot list
//!     => { <body> }                        // expression of type Option<T>
//! }
//!
//! // cases form: number/length operand unification (`as` supplies the
//! // fn name reported by the operand unifier)
//! parse_args! {
//!     fn <gen>(name: &str, args: &[Vec<CssValue>], text_flow_style: &TextFlowStyle
//!              [, <extra>: <Type>]* [,])
//!     as <fn_name_ident>
//!     -> <Result<T, ParseError>>
//!     { <ident>: <Type> [, ...] }
//!     number { <body> } length { <body> }
//! }
//! ```
//!
//! Slot types resolve through their [`ArgResolver`] impl (`CalcValue`,
//! `f32`, `RoundingStrategy`, …), so the macro carries no per-type
//! rules. The expansion yields `Option<T>`; the call site's
//! `report_parse_error` decides what gets logged.
//!
//! [`ArgResolver`]: super::args::ArgResolver

macro_rules! parse_args {
    // ── cases form: fixed or variadic slots + number/length operand
    //    unification ──
    (fn $fname:ident (
        $name:ident : & str ,
        $args:ident : & [ Vec < CssValue > ] ,
        $tfs:ident : & TextFlowStyle
        $(, $p:ident : $pty:ty)* $(,)?
    ) as $fnid:ident -> $ret:ty { $($slots:tt)* }
        number $num:tt length $len:tt) => ({
        fn $fname(
            $name: & str,
            $args: & [ Vec < CssValue > ],
            $tfs: & TextFlowStyle,
            $($p: $pty),*
        ) -> $ret {
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
        }
        $fname($name, $args, $tfs $(, $p)*)
    });
    // ── plain form: fixed or variadic slots, no unification ──
    (fn $fname:ident (
        $name:ident : & str ,
        $args:ident : & [ Vec < CssValue > ] ,
        $tfs:ident : & TextFlowStyle
        $(, $p:ident : $pty:ty)* $(,)?
    ) -> $ret:ty { $($slots:tt)* } => $body:tt) => ({
        fn $fname(
            $name: & str,
            $args: & [ Vec < CssValue > ],
            $tfs: & TextFlowStyle,
            $($p: $pty),*
        ) -> $ret {
            let mut __it = CssGroupCursor::new($args);
            munch!(
                [()]
                [__it, $name, $tfs]
                [$($slots)*] [plain $body]
            )
        }
        $fname($name, $args, $tfs $(, $p)*)
    });
}
