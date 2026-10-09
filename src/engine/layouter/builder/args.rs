//! Generic argument sequence decoder primitives.

use std::fmt;

use crate::engine::layouter::types::TextFlowStyle;

/// Resolve one comma group into a slot value for a `parse_args!` slot.
///
/// `resolve` peeks at one group without consuming it; whether the group
/// is consumed (and how failures surface) is the slot's cardinality,
/// decided by `munch!`.
pub trait ArgResolver {
    fn resolve(
        name: &str,
        text_flow_style: &TextFlowStyle,
        group: &[crate::engine::css::values::CssValue],
    ) -> Option<Self>
    where
        Self: Sized;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    MissingArgument,
    UnexpectedArgument,
    InvalidValue,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::MissingArgument => write!(f, "missing argument"),
            ParseError::UnexpectedArgument => write!(f, "unexpected argument"),
            ParseError::InvalidValue => write!(f, "invalid value"),
        }
    }
}

impl std::error::Error for ParseError {}

pub struct CssGroupCursor<'a> {
    items: &'a [Vec<crate::engine::css::values::CssValue>],
    pos: usize,
}

impl<'a> CssGroupCursor<'a> {
    pub fn new(items: &'a [Vec<crate::engine::css::values::CssValue>]) -> Self {
        Self { items, pos: 0 }
    }

    pub fn peek(&self) -> Option<&'a [crate::engine::css::values::CssValue]> {
        self.items.get(self.pos).map(|v| v.as_slice())
    }

    pub fn next(&mut self) -> Option<&'a [crate::engine::css::values::CssValue]> {
        let v = self.items.get(self.pos)?;
        self.pos += 1;
        Some(v.as_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.items.len()
    }

    /// Consume the current group without inspecting it.
    pub fn advance(&mut self) {
        if !self.is_empty() {
            self.pos += 1;
        }
    }
}
