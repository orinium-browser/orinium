//! HTMLパーサー。トークンストリームをDOMツリーに変換する。

use crate::html::tokenizer::{Attribute, Token, Tokenizer};
use crate::html::util as html_util;
use crate::{
    css::{
        matcher::{ElementChain, ElementInfo},
        parser::{CssNodeType, Parser as CssParser},
    },
    tree::{NodeRef, Tree, TreeNode},
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
#[derive(Debug, Clone)]
pub enum HtmlNodeType {
    Document,
    DocumentFragment,
    Element {
        tag_name: String,
        attributes: Vec<Attribute>,
    },
    Text(String),
    Comment(String),
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
    },
    InvalidNode(Token, String), // 不正なトークン用
}

impl HtmlNodeType {
    pub fn tag_name(&self) -> Option<&str> {
        match self {
            HtmlNodeType::Element { tag_name, .. } => Some(tag_name),
            _ => None,
        }
    }

    pub fn get_attr(&self, name: &str) -> Option<&str> {
        match self {
            HtmlNodeType::Element { attributes, .. } => attributes
                .iter()
                .find(|attr| attr.name == name)
                .map(|attr| attr.value.as_str()),
            _ => None,
        }
    }
    pub fn set_attr(&mut self, name: &str, value: String) {
        if let HtmlNodeType::Element { attributes, .. } = self {
            if let Some(attr) = attributes.iter_mut().find(|attr| attr.name == name) {
                attr.value = value;
            } else {
                attributes.push(Attribute {
                    name: name.to_string(),
                    value,
                });
            }
        }
    }
    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        if let HtmlNodeType::Element { attributes, .. } = self {
            attributes
                .iter()
                .position(|attr| attr.name == name)
                .map(|pos| attributes.remove(pos).value)
        } else {
            None
        }
    }
    pub fn has_attr(&self, name: &str) -> bool {
        match self {
            HtmlNodeType::Element { attributes, .. } => {
                attributes.iter().any(|attr| attr.name == name)
            }
            _ => false,
        }
    }
}

pub type DomTree = Tree<HtmlNodeType>;

/// Source of a classic JavaScript script in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassicScriptSource {
    Inline(String),
    External(String),
}

/// Whether scripting is enabled while parsing.
///
/// Browsers run scripts by default, so the default mode is [`ScriptingMode::Enabled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptingMode {
    #[default]
    Enabled,
    Disabled,
}

/// Scheduling mode selected by attributes on a classic script element.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClassicScriptExecution {
    #[default]
    Default,
    Defer,
    Async,
}

/// A classic script source together with its requested scheduling mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicScriptDescriptor {
    pub source: ClassicScriptSource,
    pub execution: ClassicScriptExecution,
}

impl DomTree {
    /// Returns all elements with the given tag name
    pub fn get_elements_by_tag_name(&self, tag_name: &str) -> Vec<NodeRef<HtmlNodeType>> {
        self.find_all(|n| {
            if let HtmlNodeType::Element { tag_name: t, .. } = n {
                t.eq_ignore_ascii_case(tag_name)
            } else {
                false
            }
        })
    }

    /// Returns the element with the given id
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeRef<HtmlNodeType>> {
        self.find_all(|n| {
            if let HtmlNodeType::Element { attributes, .. } = n {
                attributes
                    .iter()
                    .any(|attr| attr.name == "id" && attr.value == id)
            } else {
                false
            }
        })
        .into_iter()
        .next()
    }

    /// Returns all elements that have the given class
    pub fn get_elements_by_class_name(&self, class_name: &str) -> Vec<NodeRef<HtmlNodeType>> {
        self.find_all(|n| {
            if let HtmlNodeType::Element { attributes, .. } = n {
                attributes.iter().any(|attr| {
                    attr.name == "class" && attr.value.split_whitespace().any(|c| c == class_name)
                })
            } else {
                false
            }
        })
    }

    /// Returns the concatenated text content of this node (including children)
    pub fn inner_text(node: &NodeRef<HtmlNodeType>) -> String {
        let n = node.borrow();
        match &n.value {
            HtmlNodeType::Text(content) => content.clone(),
            HtmlNodeType::Element { .. } => n.children().iter().map(DomTree::inner_text).collect(),
            _ => "".to_string(),
        }
    }

    /// Replace all text content of this node with the given string
    pub fn set_text_content(node: &NodeRef<HtmlNodeType>, new_text: &str) {
        // Do not hold a borrow across child mutations (would double-borrow).
        if let HtmlNodeType::Text(content) = &mut node.borrow_mut().value {
            *content = new_text.to_string();
            return;
        }

        // `node.borrow()` must end before mutating below, so evaluate the
        // condition in its own statement.
        let is_element = matches!(node.borrow().value, HtmlNodeType::Element { .. });
        if is_element {
            // remove all children and add a single Text node
            node.borrow_mut().clear_children();
            let text_node = TreeNode::new(HtmlNodeType::Text(new_text.to_string()));
            TreeNode::add_child(node, text_node);
        }
    }

    /// 指定したタグ名の要素のテキストノードをすべて集める
    pub fn collect_text_by_tag(&self, tag_name: &str) -> Vec<String> {
        let mut texts = Vec::new();

        self.traverse(|node| {
            let n = node.borrow();
            if let HtmlNodeType::Element { tag_name: t, .. } = &n.value
                && t.eq_ignore_ascii_case(tag_name)
            {
                let text_of_this_node: String = n
                    .children()
                    .iter()
                    .filter_map(|child| {
                        let child_ref = child.borrow();
                        if let HtmlNodeType::Text(content) = &child_ref.value {
                            Some(content.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                texts.push(text_of_this_node);
            }
        });

        texts
    }

    /// Collects classic scripts in document order.
    ///
    /// Module scripts and data blocks with a non-JavaScript MIME type are not
    /// classic scripts and are ignored here.
    pub fn collect_classic_scripts(&self) -> Vec<ClassicScriptSource> {
        self.collect_classic_script_descriptors()
            .into_iter()
            .map(|script| script.source)
            .collect()
    }

    /// Returns the first element matching `selector` in document order.
    pub fn query_selector(&self, selector: &str) -> Option<NodeRef<HtmlNodeType>> {
        self.query_selector_all(selector).into_iter().next()
    }

    /// Returns all elements matching `selector` in document order.
    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeRef<HtmlNodeType>> {
        query_selector_all_from(&self.root, selector, true)
    }

    /// Returns the first matching descendant of `scope` in document order.
    ///
    /// The scope element itself is not considered, matching Element's DOM API.
    pub fn query_selector_within(
        scope: &NodeRef<HtmlNodeType>,
        selector: &str,
    ) -> Option<NodeRef<HtmlNodeType>> {
        Self::query_selector_all_within(scope, selector)
            .into_iter()
            .next()
    }

    /// Returns all matching descendants of `scope` in document order.
    ///
    /// The scope element itself is not included in the result.
    pub fn query_selector_all_within(
        scope: &NodeRef<HtmlNodeType>,
        selector: &str,
    ) -> Vec<NodeRef<HtmlNodeType>> {
        query_selector_all_from(scope, selector, false)
    }

    /// Returns `true` if the element itself matches the given CSS selector.
    pub fn element_matches_selector(node: &NodeRef<HtmlNodeType>, selector: &str) -> bool {
        let selectors = parse_query_selectors(selector);
        if selectors.is_empty() {
            return false;
        }
        let chain = element_chain(node);
        selectors.iter().any(|s| s.matches(&chain))
    }

    /// Walks ancestors starting from `node` and returns the first ancestor
    /// (including the node itself) that matches the given CSS selector.
    pub fn element_closest(
        node: &NodeRef<HtmlNodeType>,
        selector: &str,
    ) -> Option<NodeRef<HtmlNodeType>> {
        let selectors = parse_query_selectors(selector);
        if selectors.is_empty() {
            return None;
        }

        let mut current = Some(Rc::clone(node));
        while let Some(n) = current.clone() {
            let chain = element_chain(&n);
            if selectors.iter().any(|s| s.matches(&chain)) {
                return Some(Rc::clone(&n));
            }
            current = n.borrow().parent();
        }

        None
    }

    /// Collects classic scripts and their scheduling attributes in document order.
    pub fn collect_classic_script_descriptors(&self) -> Vec<ClassicScriptDescriptor> {
        self.get_elements_by_tag_name("script")
            .into_iter()
            .filter_map(|node| {
                let n = node.borrow();
                let script_type = n.value.get_attr("type").unwrap_or("").trim();
                if !is_classic_javascript_type(script_type) {
                    return None;
                }

                match n.value.get_attr("src").map(str::trim) {
                    Some(src) if !src.is_empty() => Some(ClassicScriptDescriptor {
                        source: ClassicScriptSource::External(src.to_string()),
                        execution: if n.value.has_attr("async") {
                            ClassicScriptExecution::Async
                        } else if n.value.has_attr("defer") {
                            ClassicScriptExecution::Defer
                        } else {
                            ClassicScriptExecution::Default
                        },
                    }),
                    Some(_) => None,
                    None => Some(ClassicScriptDescriptor {
                        source: ClassicScriptSource::Inline(DomTree::inner_text(&node)),
                        // `async` and `defer` have no effect on inline classic scripts.
                        execution: ClassicScriptExecution::Default,
                    }),
                }
            })
            .collect()
    }

    /// Collects only inline classic scripts.
    ///
    /// Kept for callers that do not yet fetch external script resources.
    pub fn collect_inline_scripts(&self) -> Vec<String> {
        self.collect_classic_scripts()
            .into_iter()
            .filter_map(|script| match script {
                ClassicScriptSource::Inline(source) => Some(source),
                ClassicScriptSource::External(_) => None,
            })
            .collect()
    }
}

fn query_selector_all_from(
    scope: &NodeRef<HtmlNodeType>,
    selector: &str,
    include_scope: bool,
) -> Vec<NodeRef<HtmlNodeType>> {
    let selectors = parse_query_selectors(selector);
    if selectors.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    collect_element_nodes(scope, include_scope, &mut candidates);
    candidates
        .into_iter()
        .filter(|node| {
            let chain = element_chain(node);
            selectors.iter().any(|selector| selector.matches(&chain))
        })
        .collect()
}

fn parse_query_selectors(selector: &str) -> Vec<crate::css::parser::ComplexSelector> {
    if selector.trim().is_empty() {
        return Vec::new();
    }

    let source = format!("{selector} {{}} ");
    let Ok(stylesheet) = CssParser::new(&source).parse() else {
        return Vec::new();
    };
    stylesheet
        .children()
        .iter()
        .find_map(|node| match node.node() {
            CssNodeType::Rule { selectors } => Some(selectors.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn collect_element_nodes(
    node: &NodeRef<HtmlNodeType>,
    include_node: bool,
    output: &mut Vec<NodeRef<HtmlNodeType>>,
) {
    let (is_element, children) = {
        let node = node.borrow();
        (
            matches!(node.value, HtmlNodeType::Element { .. }),
            node.children().to_vec(),
        )
    };
    if include_node && is_element {
        output.push(Rc::clone(node));
    }
    for child in children {
        collect_element_nodes(&child, true, output);
    }
}

fn element_chain(node: &NodeRef<HtmlNodeType>) -> ElementChain {
    let mut chain = Vec::new();
    let mut current = Some(Rc::clone(node));
    while let Some(node) = current {
        if let Some(info) = element_info(&node) {
            chain.push(info);
        }
        current = node.borrow().parent();
    }
    ElementChain::from_vec(chain)
}

fn element_info(node: &NodeRef<HtmlNodeType>) -> Option<ElementInfo> {
    let (tag_name, attributes, parent) = {
        let node = node.borrow();
        let HtmlNodeType::Element {
            tag_name,
            attributes,
        } = &node.value
        else {
            return None;
        };
        (tag_name.clone(), attributes.clone(), node.parent())
    };

    let siblings = parent
        .map(|parent| parent.borrow().children().to_vec())
        .unwrap_or_else(|| vec![Rc::clone(node)]);
    let sibling_elements: Vec<_> = siblings
        .into_iter()
        .filter_map(|sibling| basic_element_info(&sibling).map(|info| (sibling, info)))
        .collect();
    let element_count = sibling_elements.len();
    let mut type_counts = HashMap::<String, usize>::new();
    for (_, sibling) in &sibling_elements {
        *type_counts.entry(sibling.tag_name.clone()).or_default() += 1;
    }

    let position = sibling_elements
        .iter()
        .position(|(sibling, _)| Rc::ptr_eq(sibling, node))?;
    let element_index = position + 1;
    let type_index = sibling_elements[..=position]
        .iter()
        .filter(|(_, sibling)| sibling.tag_name == tag_name)
        .count();
    let previous_siblings = ElementChain::from_document_order(
        sibling_elements[..position]
            .iter()
            .map(|(_, sibling)| sibling.clone()),
    );

    Some(ElementInfo {
        tag_name: tag_name.clone(),
        id: attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case("id"))
            .map(|attribute| attribute.value.clone()),
        classes: attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case("class"))
            .map(|attribute| {
                attribute
                    .value
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        attributes: attributes
            .into_iter()
            .map(|attribute| (attribute.name, attribute.value))
            .collect(),
        element_index,
        element_count,
        type_index,
        type_count: type_counts[&tag_name],
        previous_siblings,
    })
}

fn basic_element_info(node: &NodeRef<HtmlNodeType>) -> Option<ElementInfo> {
    let node = node.borrow();
    let HtmlNodeType::Element {
        tag_name,
        attributes,
    } = &node.value
    else {
        return None;
    };
    Some(ElementInfo {
        tag_name: tag_name.clone(),
        id: attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case("id"))
            .map(|attribute| attribute.value.clone()),
        classes: attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case("class"))
            .map(|attribute| {
                attribute
                    .value
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        attributes: attributes
            .iter()
            .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
            .collect(),
        element_index: 1,
        element_count: 1,
        type_index: 1,
        type_count: 1,
        previous_siblings: ElementChain::default(),
    })
}

fn is_classic_javascript_type(script_type: &str) -> bool {
    if script_type.is_empty() {
        return true;
    }

    matches!(
        script_type.to_ascii_lowercase().as_str(),
        "text/javascript"
            | "application/javascript"
            | "text/ecmascript"
            | "application/ecmascript"
            | "application/x-javascript"
    )
}

pub struct Parser<'a> {
    tokenizer: Tokenizer<'a>,
    tree: DomTree,
    stack: Vec<Rc<RefCell<TreeNode<HtmlNodeType>>>>,
    tag_stack: Vec<String>,
    special_text_mode: Option<String>, // script/style/noscript 用
    scripting_mode: ScriptingMode,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        let document = Tree::new(HtmlNodeType::Document);

        Self {
            tokenizer: Tokenizer::new(input),
            tree: document.clone(),
            stack: vec![document.root],
            tag_stack: vec![],
            special_text_mode: None,
            scripting_mode: ScriptingMode::default(),
        }
    }

    /// Sets the scripting mode used while parsing `<noscript>` contents.
    pub fn with_scripting_mode(mut self, mode: ScriptingMode) -> Self {
        self.scripting_mode = mode;
        self
    }

    pub fn parse(&mut self) -> DomTree {
        while let Some(token) = self.tokenizer.next_token() {
            log::debug!(target:"HtmlParser::Token" ,"Processing token: {token:?}");
            match token {
                Token::StartTag { .. } => self.handle_start_tag(token),
                Token::EndTag { .. } => self.handle_end_tag(token),
                Token::Doctype { .. } => self.handle_doctype(token),
                Token::Comment(_) => self.handle_comment(token),
                Token::Text(_) => self.handle_text(token),
            }
        }
        self.autofill_elements();

        self.tree.clone()
    }

    fn handle_start_tag(&mut self, token: Token) {
        if let Token::StartTag {
            name,
            attributes,
            self_closing,
        } = token
        {
            let mut parent = Rc::clone(self.stack.last().unwrap());
            if self.special_text_mode.is_some() {
                // TODO:
                // attributes, self_closing
                TreeNode::add_child_value(&parent, HtmlNodeType::Text(format!("<{}>", name)));
                return;
            }

            // noscript は scripting フラグに応じて特別な処理を行う
            if name == "noscript" {
                self.handle_noscript(attributes);
                return;
            }

            while self.check_start_tag_with_invalid_nesting(&name, &parent) {
                if let HtmlNodeType::Element { tag_name, .. } = &parent.borrow().value {
                    log::info!(target:"HtmlParser::AutoClosing" ,"Auto-closing tag: <{}> to allow <{}> inside it.", tag_name, name);
                    self.handle_end_tag(Token::EndTag {
                        name: tag_name.clone(),
                    });
                }
                parent = Rc::clone(self.stack.last().unwrap());
            }

            let new_node = TreeNode::add_child_value(
                &parent,
                HtmlNodeType::Element {
                    tag_name: name.clone(),
                    attributes,
                },
            );

            // script/style は special mode に
            if name == "script" || name == "style" {
                self.special_text_mode = Some(name.clone());
            }

            // HTML の void 要素は自行終了扱い（stack に push しない）
            let is_void = matches!(
                name.as_str(),
                "area"
                    | "base"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            );
            // Self-closing タグは stack に push しない
            if !self_closing && !is_void {
                self.tag_stack.push(name.clone());
                self.stack.push(new_node);
                log::debug!(target:"HtmlParser::Stack" ,"Stack len: {}, +Pushed <{}> to stack.", self.stack.len(), name);
            }
        }
    }

    fn handle_noscript(&mut self, attributes: Vec<Attribute>) {
        let in_head = self
            .stack
            .last()
            .and_then(|node| node.borrow().value.tag_name().map(str::to_string))
            .is_some_and(|tag| tag.eq_ignore_ascii_case("head"));

        if in_head {
            self.handle_noscript_in_head(attributes);
        } else {
            self.handle_noscript_in_body(attributes);
        }
    }

    /// 現時点では body と同じ扱い（scripting 有効なら raw text）で、
    /// spec の "in head noscript" 挿入モード（link/meta/style の処理など）は
    /// head の挿入モードを導入した際に実装する。
    fn handle_noscript_in_head(&mut self, attributes: Vec<Attribute>) {
        match self.scripting_mode {
            ScriptingMode::Enabled => self.parse_noscript_as_raw_text(attributes),
            ScriptingMode::Disabled => self.parse_noscript_as_html(attributes),
        }
    }

    /// scripting 有効なら raw text
    /// scripting 無効なら 通常の HTML
    fn handle_noscript_in_body(&mut self, attributes: Vec<Attribute>) {
        match self.scripting_mode {
            ScriptingMode::Enabled => self.parse_noscript_as_raw_text(attributes),
            ScriptingMode::Disabled => self.parse_noscript_as_html(attributes),
        }
    }

    /// `<noscript>` の内容を raw text としてパースする
    fn parse_noscript_as_raw_text(&mut self, attributes: Vec<Attribute>) {
        self.push_element("noscript", attributes);
        self.special_text_mode = Some("noscript".to_string());
    }

    /// `<noscript>` の内容を通常の HTML としてパースする
    fn parse_noscript_as_html(&mut self, attributes: Vec<Attribute>) {
        self.push_element("noscript", attributes);
    }

    /// 要素を生成して stack に push する。
    fn push_element(&mut self, name: &str, attributes: Vec<Attribute>) {
        let parent = Rc::clone(self.stack.last().unwrap());
        let node = TreeNode::add_child_value(
            &parent,
            HtmlNodeType::Element {
                tag_name: name.to_string(),
                attributes,
            },
        );
        self.tag_stack.push(name.to_string());
        self.stack.push(node);
    }

    fn handle_end_tag(&mut self, token: Token) {
        if let Token::EndTag { ref name } = token {
            // special mode を解除
            if self.special_text_mode.as_deref() == Some(name.as_str()) {
                self.special_text_mode = None;
            }

            if self.special_text_mode.is_some() {
                let parent = Rc::clone(self.stack.last().unwrap());
                TreeNode::add_child_value(&parent, HtmlNodeType::Text(format!("</{}>", name)));
                return;
            }

            let name = name.clone();
            if self.tag_stack.contains(&name) {
                while let Some(top) = self.stack.pop() {
                    if let HtmlNodeType::Element { tag_name, .. } = &top.borrow().value {
                        self.tag_stack.pop();
                        if tag_name == &name {
                            log::debug!(target:"HtmlParser::Stack" ,"Stack len: {}, -Popped </{}> from stack.", self.stack.len(), name);
                            break;
                        } else {
                            log::debug!(target:"HtmlParser::Stack" ,"Stack len: {}, Unmatched end tag: </{}>, Find <{}>", self.stack.len(), name, tag_name);
                        }
                    }
                }
            } else {
                let parent = Rc::clone(self.stack.last().unwrap());
                TreeNode::add_child_value(
                    &parent,
                    HtmlNodeType::InvalidNode(
                        token,
                        format!("No matching start tag for </{}>", name),
                    ),
                );
                log::debug!(target:"HtmlParser::Invalid" ,"Invalid end tag: </{}>", name);
            }
        }
    }

    fn handle_text(&mut self, token: Token) {
        if let Token::Text(data) = token {
            let parent = Rc::clone(self.stack.last().unwrap());

            // special mode 中はそのままテキスト追加
            if self.special_text_mode.is_some() {
                TreeNode::add_child_value(&parent, HtmlNodeType::Text(data));
                return;
            }

            TreeNode::add_child_value(&parent, HtmlNodeType::Text(data));
        }
    }

    fn handle_comment(&mut self, token: Token) {
        if let Token::Comment(data) = token {
            let parent = Rc::clone(self.stack.last().unwrap());
            TreeNode::add_child_value(&parent, HtmlNodeType::Comment(data));
        }
    }

    fn handle_doctype(&mut self, token: Token) {
        if let Token::Doctype {
            name,
            public_id,
            system_id,
            ..
        } = token
        {
            let parent = Rc::clone(self.stack.last().unwrap());
            TreeNode::add_child_value(
                &parent,
                HtmlNodeType::Doctype {
                    name,
                    public_id,
                    system_id,
                },
            );
        }
    }

    fn check_start_tag_with_invalid_nesting(
        &self,
        name: &String,
        parent: &Rc<RefCell<TreeNode<HtmlNodeType>>>,
    ) -> bool {
        if let HtmlNodeType::Element { tag_name, .. } = &parent.borrow().value {
            // <html> 以外の中に <body> が来た場合、そのタグを閉じる
            if tag_name != "html" && name == "body" {
                println!("here we can see 「お行儀の悪いコード」");
                return true;
            }
            // <p> の中に <p> が来た場合、前の <p> を閉じる
            if tag_name == "p" && name == "p" {
                return true;
            }
            // <li> の中に <li> が来た場合、前の <li> を閉じる
            if tag_name == "li" && name == "li" {
                return true;
            }
            // <a> の中に <a> が来た場合、前の <a> を閉じる
            if tag_name == "a" && name == "a" {
                return true;
            }
            // <dt> の中に <dt> または <dd> が来た場合、前の <dt> を閉じる
            if tag_name == "dt" && (name == "dt" || name == "dd") {
                return true;
            }
            // <dd> の中に <dt> または <dd> が来た場合、前の <dd> を閉じる
            if tag_name == "dd" && (name == "dt" || name == "dd") {
                return true;
            }
            // <option> の中に <option> が来た場合、前の <option> を閉じる
            if tag_name == "option" && name == "option" {
                return true;
            }
            // <p> の中にブロック要素が来た場合、前の <p> を閉じる
            if matches!(
                tag_name.as_str(),
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) && html_util::is_block_level_element(name)
            {
                return true;
            }
        }
        false
    }

    /// DOCTYPE宣言、html, head, body 要素が存在しない場合に補完する
    fn autofill_elements(&mut self) {
        let root = Rc::clone(&self.stack[0]);
        let has_html = root
            .borrow()
            .children()
            .iter()
            .any(|c| matches!(&c.borrow().value, HtmlNodeType::Element { tag_name, .. } if tag_name.to_lowercase() == "html"));

        if !has_html {
            let mut doctype_node = None;
            let mut orphan_nodes = Vec::new();
            for child in root.borrow().children() {
                match &child.borrow().value {
                    HtmlNodeType::Doctype { .. } => {
                        doctype_node = Some(Rc::clone(child));
                    }
                    _ => orphan_nodes.push(Rc::clone(child)),
                }
            }

            root.borrow_mut().clear_children();

            if let Some(dt) = doctype_node {
                TreeNode::add_child(&root, dt);
            } else {
                TreeNode::add_child_value(
                    &root,
                    HtmlNodeType::Doctype {
                        name: Some("html".to_string()),
                        public_id: None,
                        system_id: None,
                    },
                );
            }

            let html_node = TreeNode::add_child_value(
                &root,
                HtmlNodeType::Element {
                    tag_name: "html".to_string(),
                    attributes: vec![],
                },
            );

            TreeNode::add_child_value(
                &html_node,
                HtmlNodeType::Element {
                    tag_name: "head".to_string(),
                    attributes: vec![],
                },
            );

            let body_node = TreeNode::add_child_value(
                &html_node,
                HtmlNodeType::Element {
                    tag_name: "body".to_string(),
                    attributes: vec![],
                },
            );

            for orphan in orphan_nodes {
                TreeNode::add_child(&body_node, orphan);
            }
        }
    }
}
