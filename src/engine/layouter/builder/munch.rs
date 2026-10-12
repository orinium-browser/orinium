//! Generic slot-munching core for `parse_args!` slot lists.
//!
//! Walks a [`CssGroupCursor`](super::args::CssGroupCursor) one comma
//! group at a time and resolves each through the [`ArgResolver`] impl of
//! the declared slot type. The cursor binding, `name`, and
//! `text_flow_style` are forwarded as metavariables (locals are
//! hygienic across macro expansions, so the cursor cannot be shared
//! otherwise); `ArgResolver` itself resolves at the call site, which
//! must import it anyway for its slot impls.
//!
//! Slot forms: `x: T` (required), `x: T = <expr>` (default),
//! `x: Option<T>` (try-parse: consume only on success), `x: Vec<T>`
//! (variadic, last slot only).
//!
//! ```text
//! munch! [cursor, name, text_flow_style] [<slots...>] [plain <body>]
//! ```
//!
//! Evaluates to `Result<T, ParseError>`; arity failures become
//! `MissingArgument`/`UnexpectedArgument`, a body `None` becomes
//! `InvalidValue`. Logging stays the call site's decision.
//!
//! [`ArgResolver`]: super::args::ArgResolver

macro_rules! munch {
    // ── slot muncher: resolve one comma group per slot, in order ──
    // Env: [cursor, name, text_flow_style]. Failures are plain `Result`
    // values; the call-site bridge decides whether to log them.

    // Try-parse slot (any position): consume the peeked group only when
    // it resolves; never contributes to later slots otherwise.
    ([$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : Option<$inner:ty> $(, $($rest:tt)*)?] $sfx:tt) => {{
        let $ident: Option<$inner> = $it
            .peek()
            .and_then(|group| <$inner as ArgResolver>::resolve($name, $tfs, group));
        if $ident.is_some() {
            $it.advance();
        }
        munch!([$it, $name, $tfs] [$($($rest)*)?] $sfx)
    }};
    // Variadic slot (last): collect every remaining group.
    ([$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : Vec<$inner:ty> $(,)?] [plain $body:tt]) => {
        if $it.is_empty() {
            Err(ParseError::MissingArgument)
        } else {
            let $ident: Vec<$inner> = std::iter::from_fn(|| $it.next())
                .map(|group| <$inner as ArgResolver>::resolve($name, $tfs, group))
                .collect::<Option<Vec<$inner>>>()
                .ok_or(ParseError::InvalidValue)?;
            munch!([$it, $name, $tfs] [] [plain $body])
        }
    };
    // Take-if-present slot (mid): absent binds the default, a present
    // but unresolvable group is an error.
    ([$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : $ty:ty = $default:expr, $($rest:tt)*] $sfx:tt) => {{
        let $ident: $ty = match $it.next() {
            Some(group) => <$ty as ArgResolver>::resolve($name, $tfs, group)
                .ok_or(ParseError::InvalidValue)?,
            None => $default,
        };
        munch!([$it, $name, $tfs] [$($rest)*] $sfx)
    }};
    // Take-if-present slot (last).
    ([$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : $ty:ty = $default:expr] $sfx:tt) => {{
        let $ident: $ty = match $it.next() {
            Some(group) => <$ty as ArgResolver>::resolve($name, $tfs, group)
                .ok_or(ParseError::InvalidValue)?,
            None => $default,
        };
        munch!([$it, $name, $tfs] [] $sfx)
    }};
    // Required slot (mid).
    ([$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : $ty:ty, $($rest:tt)*] $sfx:tt) => {
        match $it.next() {
            Some(group) => {
                let $ident: $ty = <$ty as ArgResolver>::resolve($name, $tfs, group)
                    .ok_or(ParseError::InvalidValue)?;
                munch!([$it, $name, $tfs] [$($rest)*] $sfx)
            }
            None => Err(ParseError::MissingArgument),
        }
    };
    // Required slot (last).
    ([$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : $ty:ty] $sfx:tt) => {
        match $it.next() {
            Some(group) => {
                let $ident: $ty = <$ty as ArgResolver>::resolve($name, $tfs, group)
                    .ok_or(ParseError::InvalidValue)?;
                munch!([$it, $name, $tfs] [] $sfx)
            }
            None => Err(ParseError::MissingArgument),
        }
    };
    // All slots consumed: reject leftover groups, run the plain body.
    ([$it:ident, $name:ident, $tfs:ident]
        [] [plain $body:tt]) => {
        if $it.next().is_some() {
            Err(ParseError::UnexpectedArgument)
        } else {
            match $body {
                Some(v) => Ok(v),
                None => Err(ParseError::InvalidValue),
            }
        }
    };
}
