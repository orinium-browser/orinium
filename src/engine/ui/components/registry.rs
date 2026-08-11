//! Component registry: maps HTML tags to custom node factories.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use crate::engine::bridge::text;
use crate::engine::layouter::normalize_whitespace;
use crate::engine::layouter::types::{Color, ContainerStyle, TextStyle};
use crate::engine::layouter::{DomSnapshot, NodeId};
use crate::engine::renderer_model::Image;
use crate::engine::ui::audio::AudioComponent;
use crate::engine::ui::button::ButtonComponent;
use crate::engine::ui::components::input_hidden::InputHiddenComponent;
use crate::engine::ui::custom_node::CustomNode;
use crate::engine::ui::image::ImageComponent;
use crate::engine::ui::input_text::InputTextComponent;
use crate::engine::ui::input_text::OnValueChange;
use crate::engine::ui::select::{OnSelectChange, SelectComponent, SelectOption};

/// A channel for reporting text-input value changes to the DOM owner.
///
/// Layout is built off the UI thread on a [`DomSnapshot`]-style arena, so the
/// builder cannot touch the real DOM. Text inputs instead report
/// `(snapshot node id, new value)` through this channel; the UI thread drains
/// it and applies the value to the live tree.
pub type DomWriteBack = mpsc::Sender<(u32, String)>;

/// Context handed to a [`CustomNodeFactory`] to construct a component.
pub struct CustomNodeContext<'a> {
    /// The HTML tag name (e.g. `"button"`).
    pub tag: &'a str,
    /// Inner text of the element, if any.
    pub media_source: Option<&'a str>,
    /// Resolved container style (background, border, …).
    pub container_style: &'a ContainerStyle,
    /// Inherited text style.
    pub text_style: &'a TextStyle,
    /// Text measurer for text-heavy components.
    pub measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    /// Decoded images keyed by `src` URL.
    pub images: &'a HashMap<String, Image>,
    /// Encoded audio bytes keyed by `src` URL.
    pub audio: &'a HashMap<String, Arc<[u8]>>,
    /// Attribute accessor.
    pub get_attr: &'a dyn Fn(&str) -> Option<String>,
    /// Channel + snapshot node id for value write-back (bidirectional sync).
    pub write_back: Option<(DomWriteBack, u32)>,
    /// Dom snapshot.
    pub dom_snapshot: &'a DomSnapshot,
    /// The node id.
    pub dom_id: NodeId,
}

/// Constructs a [`CustomNode`] for a given HTML tag.
pub trait CustomNodeFactory {
    /// The tags this factory can construct (drives `CUSTOM_TAGS`).
    fn tags(&self) -> &'static [&'static str];

    /// Builds a node for `tag`, or `None` if the tag is not handled here.
    fn create(&self, tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>>;
}

/// A registry of [`CustomNodeFactory`]es used by the layout builder.
#[derive(Default)]
pub struct ComponentRegistry {
    factories: Vec<Box<dyn CustomNodeFactory>>,
}

impl ComponentRegistry {
    /// Creates a registry with the built-in components registered.
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register(Box::new(ButtonFactory));
        registry.register(Box::new(AudioFactory));
        registry.register(Box::new(ImageFactory));
        registry.register(Box::new(InputTextFactory));
        registry.register(Box::new(SelectFactory));
        registry
    }

    /// Registers a factory, replacing any factory with the same tags.
    pub fn register(&mut self, factory: Box<dyn CustomNodeFactory>) {
        self.factories
            .retain(|f| factory.tags().iter().all(|tag| !f.tags().contains(tag)));
        self.factories.push(factory);
    }

    /// All tags the registry can construct.
    pub fn tags(&self) -> Vec<&'static str> {
        self.factories
            .iter()
            .flat_map(|f| f.tags().iter().copied())
            .collect()
    }

    /// Constructs a node for `tag`, or `None` if no factory handles it.
    pub fn create(&self, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        for factory in &self.factories {
            if factory.tags().contains(&ctx.tag) {
                return factory.create(ctx.tag, ctx);
            }
        }
        None
    }
}

struct AudioFactory;

impl CustomNodeFactory for AudioFactory {
    fn tags(&self) -> &'static [&'static str] {
        &["audio"]
    }

    fn create(&self, _tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        Some(Arc::new(AudioComponent::new(
            ctx.media_source.unwrap_or_default(),
            ctx.media_source
                .and_then(|source| ctx.audio.get(source))
                .cloned(),
        )))
    }
}

struct ButtonFactory;

impl CustomNodeFactory for ButtonFactory {
    fn tags(&self) -> &'static [&'static str] {
        &["button"]
    }

    fn create(&self, _tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        let default_bg = Color(240, 240, 240, 255);
        let bg = match &ctx.container_style.background {
            crate::engine::layouter::types::Background::Color(c) if c.3 > 0 => *c,
            _ => default_bg,
        };
        Some(Arc::new(ButtonComponent::new(
            ctx.dom_snapshot.inner_text(ctx.dom_id),
            bg,
            ctx.text_style.color,
            Arc::clone(&ctx.measurer),
        )))
    }
}

struct ImageFactory;

impl CustomNodeFactory for ImageFactory {
    fn tags(&self) -> &'static [&'static str] {
        &["img"]
    }

    fn create(&self, _tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        let image = (ctx.get_attr)("src")
            .and_then(|source| ctx.images.get(&source))
            .cloned();
        Some(Arc::new(ImageComponent::new(
            image,
            (ctx.get_attr)("alt").unwrap_or_default(),
        )))
    }
}

struct InputTextFactory;

impl CustomNodeFactory for InputTextFactory {
    fn tags(&self) -> &'static [&'static str] {
        &["input"]
    }

    fn create(&self, _tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        let type_ = (ctx.get_attr)("type").unwrap_or_default();
        let value = (ctx.get_attr)("value").unwrap_or_default();
        let placeholder = (ctx.get_attr)("placeholder").unwrap_or_default();

        if type_.eq_ignore_ascii_case("hidden") {
            return Some(Arc::new(InputHiddenComponent::new(value)));
        }

        let on_value_change = ctx.write_back.as_ref().map(|(sender, node_id)| {
            let sender = sender.clone();
            let node_id = *node_id;
            Arc::new(move |new_value: &str| {
                let _ = sender.send((node_id, new_value.to_string()));
            }) as Arc<OnValueChange>
        });

        Some(Arc::new(if let Some(cb) = on_value_change {
            InputTextComponent::with_on_change(value, placeholder, Arc::clone(&ctx.measurer), cb)
        } else {
            InputTextComponent::new(value, placeholder, Arc::clone(&ctx.measurer))
        }))
    }
}

struct SelectFactory;

impl CustomNodeFactory for SelectFactory {
    fn tags(&self) -> &'static [&'static str] {
        &["select"]
    }

    fn create(&self, _tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        let value = (ctx.get_attr)("value").unwrap_or_default();
        let disabled = (ctx.get_attr)("disabled").is_some();
        let multiple = (ctx.get_attr)("multiple").is_some();
        let on_change = ctx.write_back.as_ref().map(|(sender, node_id)| {
            let sender = sender.clone();
            let node_id = *node_id;
            Arc::new(move |new_value: &str| {
                let _ = sender.send((node_id, new_value.to_string()));
            }) as Arc<OnSelectChange>
        });

        let mut options: Vec<SelectOption> = Vec::new();
        for id in ctx.dom_snapshot.children(ctx.dom_id) {
            let node = ctx.dom_snapshot.node(*id);
            match node.kind.tag_name() {
                Some("optgroup") => {
                    let group = node.kind.get_attr("label").unwrap_or_default().to_string();
                    let group_disabled = node.kind.has_attr("disabled");
                    for child in ctx.dom_snapshot.children(*id) {
                        let child_node = ctx.dom_snapshot.node(*child);
                        if child_node.kind.tag_name() != Some("option") {
                            continue;
                        }
                        options.push(SelectOption {
                            value: child_node
                                .kind
                                .get_attr("value")
                                .unwrap_or_default()
                                .to_string(),
                            label: normalize_whitespace(&ctx.dom_snapshot.inner_text(*child), true),
                            selected: child_node.kind.has_attr("selected"),
                            disabled: group_disabled || child_node.kind.has_attr("disabled"),
                            group: Some(group.clone()),
                        });
                    }
                }
                Some("option") => options.push(SelectOption {
                    value: node.kind.get_attr("value").unwrap_or_default().to_string(),
                    label: normalize_whitespace(&ctx.dom_snapshot.inner_text(*id), true),
                    selected: node.kind.has_attr("selected"),
                    disabled: node.kind.has_attr("disabled"),
                    group: None,
                }),
                _ => {}
            }
        }

        Some(Arc::new(if let Some(cb) = on_change {
            SelectComponent::with_on_change(
                options,
                &value,
                Arc::clone(&ctx.measurer),
                cb,
                disabled,
                multiple,
            )
        } else {
            SelectComponent::new(
                options,
                &value,
                Arc::clone(&ctx.measurer),
                disabled,
                multiple,
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;
    use crate::engine::html::parser::DomTree;
    use crate::engine::html::parser::Parser as HtmlParser;
    use crate::engine::layouter::DomSnapshot;
    use crate::engine::ui::custom_node::PointerEvent;

    fn tree(html: &str) -> DomTree {
        HtmlParser::new(html).parse()
    }

    fn empty_snapshot() -> DomSnapshot {
        let dom = tree("<html></html>");
        let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
        snapshot
    }

    #[test]
    fn registry_builds_known_components() {
        let registry = ComponentRegistry::new();
        let mut attrs = HashMap::new();
        attrs.insert("value".to_string(), "abc".to_string());
        attrs.insert("placeholder".to_string(), "ph".to_string());
        attrs.insert("src".to_string(), "img.png".to_string());
        let container_style = ContainerStyle::default();
        let text_style = TextStyle::default();
        let images = HashMap::new();
        let audio = HashMap::new();
        let get_attr = |name: &str| attrs.get(name).cloned();
        let measurer: Arc<dyn text::TextMeasurer<TextStyle>> = Arc::new(FallbackTextMeasurer);

        let dom_snapshot = &{
            let dom = tree(
                "<html><button></button><input /><img /><select><option value=\"opt\">Option</option></select></html>",
            );
            let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
            snapshot
        };

        let html_id = dom_snapshot.node(dom_snapshot.roots()[0]).children[0];

        let button_id = dom_snapshot.children(html_id)[0];
        let input_id = dom_snapshot.children(html_id)[1];
        let img_id = dom_snapshot.children(html_id)[2];
        let select_id = dom_snapshot.children(html_id)[3];

        let button = registry
            .create(&CustomNodeContext {
                tag: "button",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: button_id,
            })
            .unwrap();
        assert_eq!(button.role(), Some("button"));

        let input = registry
            .create(&CustomNodeContext {
                tag: "input",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: input_id,
            })
            .unwrap();
        assert_eq!(input.role(), Some("textbox"));
        assert_eq!(input.value(), Some("abc".to_string()));

        let img = registry
            .create(&CustomNodeContext {
                tag: "img",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: img_id,
            })
            .unwrap();
        assert_eq!(img.role(), None);

        let select = registry
            .create(&CustomNodeContext {
                tag: "select",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: select_id,
            })
            .unwrap();
        assert_eq!(select.role(), Some("combobox"));
        assert_eq!(select.value(), Some("opt".to_string()));
    }

    #[test]
    fn select_parses_optgroup_and_disabled() {
        fn find(snapshot: &DomSnapshot, id: NodeId, tag: &str) -> Option<NodeId> {
            if snapshot.node(id).kind.tag_name() == Some(tag) {
                return Some(id);
            }
            snapshot
                .children(id)
                .iter()
                .find_map(|&c| find(snapshot, c, tag))
        }

        let registry = ComponentRegistry::new();
        let mut attrs = HashMap::new();
        attrs.insert("value".to_string(), "b".to_string());
        attrs.insert("disabled".to_string(), String::new());
        let container_style = ContainerStyle::default();
        let text_style = TextStyle::default();
        let images: HashMap<String, Image> = HashMap::new();
        let audio = HashMap::new();
        let get_attr = |name: &str| attrs.get(name).cloned();

        let dom_snapshot = &{
            let dom = tree(
                "<html><select value=\"b\" disabled><optgroup label=\"Fruits\" disabled><option value=\"a\" selected>Apple</option><option value=\"b\" disabled>Banana</option></optgroup><option value=\"c\">Cherry</option></select></html>",
            );
            let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
            snapshot
        };
        let select_id = find(dom_snapshot, dom_snapshot.roots()[0], "select").unwrap();

        let select = registry
            .create(&CustomNodeContext {
                tag: "select",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::new(FallbackTextMeasurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: select_id,
            })
            .unwrap();

        // The disabled `<select>` reports disabled and never opens a popup.
        assert!(select.is_disabled());
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_none());

        // The value resolves to an option nested inside the `<optgroup>`.
        assert_eq!(select.value(), Some("b".to_string()));
        assert_eq!(select.label(), Some("Banana".to_string()));
    }

    #[test]
    fn select_optgroup_options_are_grouped() {
        fn find(snapshot: &DomSnapshot, id: NodeId, tag: &str) -> Option<NodeId> {
            if snapshot.node(id).kind.tag_name() == Some(tag) {
                return Some(id);
            }
            snapshot
                .children(id)
                .iter()
                .find_map(|&c| find(snapshot, c, tag))
        }

        let registry = ComponentRegistry::new();
        let attrs: HashMap<String, String> = HashMap::new();
        let container_style = ContainerStyle::default();
        let text_style = TextStyle::default();
        let images: HashMap<String, Image> = HashMap::new();
        let audio = HashMap::new();
        let get_attr = |name: &str| attrs.get(name).cloned();

        let dom_snapshot = &{
            let dom = tree(
                "<html><select><optgroup label=\"Fruits\"><option value=\"a\">Apple</option></optgroup><option value=\"b\">Banana</option></select></html>",
            );
            let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
            snapshot
        };
        let select_id = find(dom_snapshot, dom_snapshot.roots()[0], "select").unwrap();

        let select = registry
            .create(&CustomNodeContext {
                tag: "select",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::new(FallbackTextMeasurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: select_id,
            })
            .unwrap();

        assert!(!select.is_disabled());
        // The `<optgroup>` option is selectable and opens a popup.
        select.on_pointer_event(PointerEvent::Down { x: 5.0, y: 5.0 });
        assert!(select.popup(&TextStyle::default()).is_some());
        assert_eq!(select.value(), Some("a".to_string()));
    }

    #[test]
    fn select_parses_multiple_attribute() {
        fn find(snapshot: &DomSnapshot, id: NodeId, tag: &str) -> Option<NodeId> {
            if snapshot.node(id).kind.tag_name() == Some(tag) {
                return Some(id);
            }
            snapshot
                .children(id)
                .iter()
                .find_map(|&c| find(snapshot, c, tag))
        }

        let registry = ComponentRegistry::new();
        let mut attrs = HashMap::new();
        attrs.insert("multiple".to_string(), String::new());
        let container_style = ContainerStyle::default();
        let text_style = TextStyle::default();
        let images: HashMap<String, Image> = HashMap::new();
        let audio = HashMap::new();
        let get_attr = |name: &str| attrs.get(name).cloned();

        let dom_snapshot = &{
            let dom = tree(
                "<html><select multiple><option value=\"a\" selected>Apple</option><option value=\"b\">Banana</option><option value=\"c\" selected>Cherry</option></select></html>",
            );
            let (snapshot, _dom_refs) = DomSnapshot::from_tree(&dom.root);
            snapshot
        };
        let select_id = find(dom_snapshot, dom_snapshot.roots()[0], "select").unwrap();

        let select = registry
            .create(&CustomNodeContext {
                tag: "select",
                media_source: None,
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::new(FallbackTextMeasurer),
                images: &images,
                audio: &audio,
                get_attr: &get_attr,
                write_back: None,
                dom_snapshot,
                dom_id: select_id,
            })
            .unwrap();

        // Multiple selects render as a list box: no popup, comma-joined value.
        assert_eq!(select.role(), Some("listbox"));
        assert_eq!(select.value(), Some("a,c".to_string()));
        assert!(select.popup(&TextStyle::default()).is_none());

        // Clicking a row toggles it without opening a popup.
        select.on_pointer_event(PointerEvent::Down {
            x: 5.0,
            y: 28.0 + 2.0,
        });
        assert_eq!(select.value(), Some("a,b,c".to_string()));
    }

    #[test]
    fn registry_returns_none_for_unknown_tag() {
        let registry = ComponentRegistry::new();
        let attrs: HashMap<String, String> = HashMap::new();
        let container_style = ContainerStyle::default();
        let text_style = TextStyle::default();
        let images: HashMap<String, Image> = HashMap::new();
        let audio = HashMap::new();
        let get_attr = |name: &str| attrs.get(name).cloned();
        let ctx = CustomNodeContext {
            tag: "video",
            media_source: None,
            container_style: &container_style,
            text_style: &text_style,
            measurer: Arc::new(FallbackTextMeasurer),
            images: &images,
            audio: &audio,
            get_attr: &get_attr,
            write_back: None,
            dom_snapshot: &empty_snapshot(),
            dom_id: 0,
        };
        assert!(registry.create(&ctx).is_none());
    }

    #[test]
    fn register_is_idempotent_for_same_tag() {
        let mut registry = ComponentRegistry::new();
        let mut tags_before = registry.tags();
        tags_before.sort();
        registry.register(Box::new(ButtonFactory));
        let mut tags_after = registry.tags();
        tags_after.sort();
        assert_eq!(tags_after, tags_before);
    }
}
