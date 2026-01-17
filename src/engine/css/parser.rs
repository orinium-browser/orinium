/// Parsed CSS stylesheet.
///
/// This is an internal representation used by the style engine.
pub struct Stylesheet;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Selector {
    /// Type selector (e.g. `div`)
    pub tag: Option<String>,

    /// ID selector (e.g. `#main`)
    pub id: Option<String>,

    /// Class selectors (e.g. `.container`)
    pub classes: Vec<String>,

    /// Pseudo-class (e.g. `:hover`)
    pub pseudo_class: Option<String>,

    /// Pseudo-element (e.g. `::before`)
    pub pseudo_element: Option<String>,
}

/// Combinator that defines the relationship between this selector
/// and the next selector evaluated on the left.
///
/// In right-to-left selector matching, the combinator belongs to
/// the selector on the right-hand side.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Combinator {
    /// Descendant combinator (` `)
    Descendant,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectorPart {
    /// Simple selector to be matched at the current step
    pub selector: Selector,

    /// Relationship to the next selector on the left.
    ///
    /// `None` means this is the leftmost selector in the sequence.
    pub combinator: Option<Combinator>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComplexSelector {
    /// Selector parts ordered from right to left.
    ///
    /// Example:
    /// `A B` is stored as:
    ///   [ B (Descendant), A (None) ]
    pub parts: Vec<SelectorPart>,
}
