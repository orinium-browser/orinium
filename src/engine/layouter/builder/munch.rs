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
//! munch! [()] [cursor, name, text_flow_style(, unify)] [<slots...>] <suffix>
//! ```
//!
//! Suffix: `plain <body>` or `cases <body_number> <body_length>` — the
//! latter folds the fixed + Vec slots through the unifier; `Ok`
//! selects the number body, `Err` the length body. Bodies are opaque
//! `Option<T>` tokens evaluated in the caller's context.
//!
//! Evaluates to `Result<T, ParseError>`; arity failures become
//! `MissingArgument`/`UnexpectedArgument`, a body `None` becomes
//! `InvalidValue`. Logging stays the call site's decision.
//!
//! [`ArgResolver`]: super::args::ArgResolver

macro_rules! munch {
    // ── slot muncher: resolve one comma group per slot, in order ──
    // Env: [cursor, name, text_flow_style] plus a unifier for the
    // `cases` suffix. Failures are plain `Result` values; the call-site
    // bridge decides whether to log them.

    // Try-parse slot (any position): consume the peeked group only
    // when it resolves; never contributes to unification.
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident $(, $unify:ident)?]
        [$ident:ident : Option<$inner:ty> $(, $($rest:tt)*)?] $sfx:tt) => {
        match $it
            .peek()
            .and_then(|group| <$inner as ArgResolver>::resolve($name, $tfs, group))
        {
            Some(parsed) => {
                $it.advance();
                let $ident: Option<$inner> = Some(parsed);
                munch!([($($ids)*)] [$it, $name, $tfs $(, $unify)?] [$($($rest)*)?] $sfx)
            }
            None => {
                let $ident: Option<$inner> = None;
                munch!([($($ids)*)] [$it, $name, $tfs $(, $unify)?] [$($($rest)*)?] $sfx)
            }
        }
    };
    // Variadic slot (last) with a plain body: collect every remaining
    // group, then run the body through the plain final.
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident]
        [$ident:ident : Vec<$inner:ty> $(,)?] [plain $body:tt]) => {
        if $it.is_empty() {
            Err(ParseError::MissingArgument)
        } else {
            let $ident: Vec<$inner> = std::iter::from_fn(|| $it.next())
                .map(|group| <$inner as ArgResolver>::resolve($name, $tfs, group))
                .collect::<Option<Vec<$inner>>>()
                .ok_or(ParseError::InvalidValue)?;
            munch!([($($ids)* $ident)] [$it, $name, $tfs] [] [plain $body])
        }
    };
    // Variadic slot (last) feeding operand unification: collect every
    // remaining group, fold the fixed bindings and the Vec through the
    // unifier, then re-bind from the unified result and run the
    // selected case body.
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident, $unify:ident]
        [$ident:ident : Vec<$inner:ty> $(,)?] [cases $num:tt $len:tt]) => {
        if $it.is_empty() {
            Err(ParseError::MissingArgument)
        } else {
            let $ident: Vec<$inner> = std::iter::from_fn(|| $it.next())
                .map(|group| <$inner as ArgResolver>::resolve($name, $tfs, group))
                .collect::<Option<Vec<$inner>>>()
                .ok_or(ParseError::InvalidValue)?;
            let mut __operands = vec![$($ids),*];
            __operands.extend($ident);
            match ($unify)(__operands) {
                Some(Ok(values)) => munch!(@vec_rebind [($($ids)*)] [$ident] values, $num),
                Some(Err(values)) => munch!(@vec_rebind [($($ids)*)] [$ident] values, $len),
                None => Err(ParseError::InvalidValue),
            }
        }
    };
    // Take-if-present slot (mid): absent binds the default, a present
    // but unresolvable group is an error (the slot stays a plain `T` so
    // it can feed unification).
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident $(, $unify:ident)?]
        [$ident:ident : $ty:ty = $default:expr] $sfx:tt) => {{
        let $ident: $ty = match $it.next() {
            Some(group) => {
                <$ty as ArgResolver>::resolve($name, $tfs, group)
                    .ok_or(ParseError::InvalidValue)?
            }
            None => $default,
        };
        munch!([($($ids)* $ident)] [$it, $name, $tfs $(, $unify)?] [] $sfx)
    }};
    // Take-if-present slot (last).
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident $(, $unify:ident)?]
        [$ident:ident : $ty:ty = $default:expr] $sfx:tt) => {
        let $ident: $ty = match $it.next() {
            Some(group) => {
                <$ty as ArgResolver>::resolve($name, $tfs, group)
                    .ok_or(ParseError::InvalidValue)?
            }
            None => $default,
        };
        munch!([($($ids)* $ident)] [$it, $name, $tfs $(, $unify)?] [] $sfx)
    };
    // Required slot (mid).
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident $(, $unify:ident)?]
        [$ident:ident : $ty:ty, $($rest:tt)*] $sfx:tt) => {
        match $it.next() {
            Some(group) => {
                let $ident: $ty = <$ty as ArgResolver>::resolve($name, $tfs, group)
                    .ok_or(ParseError::InvalidValue)?;
                munch!([($($ids)* $ident)] [$it, $name, $tfs $(, $unify)?] [$($rest)*] $sfx)
            }
            None => Err(ParseError::MissingArgument),
        }
    };
    // Required slot (last).
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident $(, $unify:ident)?]
        [$ident:ident : $ty:ty] $sfx:tt) => {
        match $it.next() {
            Some(group) => {
                let $ident: $ty = <$ty as ArgResolver>::resolve($name, $tfs, group)
                    .ok_or(ParseError::InvalidValue)?;
                munch!([($($ids)* $ident)] [$it, $name, $tfs $(, $unify)?] [] $sfx)
            }
            None => Err(ParseError::MissingArgument),
        }
    };
    // All slots consumed: reject leftover groups, fold the bound values
    // through the unifier, and re-bind the slot identifiers from the
    // selected case's array before running the case body.
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident, $unify:ident]
        [] [cases $num:tt $len:tt]) => {
        if $it.next().is_some() {
            Err(ParseError::UnexpectedArgument)
        } else {
            match ($unify)(vec![$($ids),*]) {
                Some(Ok(values)) => munch!(@arr_rebind [($($ids)*)] values, $num),
                Some(Err(values)) => munch!(@arr_rebind [($($ids)*)] values, $len),
                None => Err(ParseError::InvalidValue),
            }
        }
    };
    // All slots consumed: reject leftover groups, run the plain body.
    ([($($ids:ident)*)]
        [$it:ident, $name:ident, $tfs:ident]
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

    // ── case helpers: re-bind the slot identifiers from a unified
    //    operand list, then run one of the two case bodies ──

    // Iterator form: fixed slots pop off the front, the tail collects
    // back into the Vec slot (used by the variadic cases rule).
    (@vec_rebind [($($ids:ident)*)] [$vid:ident] $values:expr, $body:tt) => {{
        #[allow(unused_mut)]
        let mut __rit = ($values).into_iter();
        $(let $ids = __rit.next().ok_or(ParseError::InvalidValue)?;)*
        let $vid: Vec<_> = __rit.collect();
        match $body {
            Some(v) => Ok(v),
            None => Err(ParseError::InvalidValue),
        }
    }};
    // Array form: every slot destructures from the fixed-size result
    // (used by the all-slots-consumed cases rule).
    (@arr_rebind [($($ids:ident)*)] $values:expr, $body:tt) => {{
        match ($values).try_into() {
            Ok(array) => {
                let [$($ids),*] = array;
                match $body {
                    Some(v) => Ok(v),
                    None => Err(ParseError::InvalidValue),
                }
            }
            Err(_) => Err(ParseError::InvalidValue),
        }
    }};
}
