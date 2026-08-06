//! Component registry: maps HTML tags to custom node factories.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use crate::engine::bridge::text;
use crate::engine::layouter::types::{Color, ContainerStyle, TextStyle};
use crate::engine::renderer_model::Image;
use crate::engine::ui::button::ButtonComponent;
use crate::engine::ui::custom_node::CustomNode;
use crate::engine::ui::image::ImageComponent;
use crate::engine::ui::text_input::OnValueChange;
use crate::engine::ui::text_input::TextInputComponent;

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
    pub inner_text: &'a str,
    /// Resolved container style (background, border, …).
    pub container_style: &'a ContainerStyle,
    /// Inherited text style.
    pub text_style: &'a TextStyle,
    /// Text measurer for text-heavy components.
    pub measurer: Arc<dyn text::TextMeasurer<TextStyle>>,
    /// Decoded images keyed by `src` URL.
    pub images: &'a HashMap<String, Image>,
    /// Attribute accessor.
    pub get_attr: &'a dyn Fn(&str) -> Option<String>,
    /// Channel + snapshot node id for value write-back (bidirectional sync).
    pub write_back: Option<(DomWriteBack, u32)>,
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
        registry.register(Box::new(ImageFactory));
        registry.register(Box::new(TextInputFactory));
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
            ctx.inner_text.to_string(),
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

struct TextInputFactory;

impl CustomNodeFactory for TextInputFactory {
    fn tags(&self) -> &'static [&'static str] {
        &["input"]
    }

    fn create(&self, _tag: &str, ctx: &CustomNodeContext) -> Option<Arc<dyn CustomNode>> {
        let value = (ctx.get_attr)("value").unwrap_or_default();
        let placeholder = (ctx.get_attr)("placeholder").unwrap_or_default();
        let on_value_change = ctx.write_back.as_ref().map(|(sender, node_id)| {
            let sender = sender.clone();
            let node_id = *node_id;
            Arc::new(move |new_value: &str| {
                let _ = sender.send((node_id, new_value.to_string()));
            }) as Arc<OnValueChange>
        });
        Some(Arc::new(if let Some(cb) = on_value_change {
            TextInputComponent::with_on_change(value, placeholder, Arc::clone(&ctx.measurer), cb)
        } else {
            TextInputComponent::new(value, placeholder, Arc::clone(&ctx.measurer))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::text::FallbackTextMeasurer;

    #[test]
    fn registry_reports_builtin_tags() {
        let tags = ComponentRegistry::new().tags();
        for expected in ["button", "img", "input"] {
            assert!(tags.contains(&expected), "missing tag {expected}");
        }
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
        let get_attr = |name: &str| attrs.get(name).cloned();
        let measurer: Arc<dyn text::TextMeasurer<TextStyle>> = Arc::new(FallbackTextMeasurer);

        let button = registry
            .create(&CustomNodeContext {
                tag: "button",
                inner_text: "",
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                get_attr: &get_attr,
                write_back: None,
            })
            .unwrap();
        assert_eq!(button.role(), Some("button"));

        let input = registry
            .create(&CustomNodeContext {
                tag: "input",
                inner_text: "",
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                get_attr: &get_attr,
                write_back: None,
            })
            .unwrap();
        assert_eq!(input.role(), Some("textbox"));
        assert_eq!(input.value(), Some("abc".to_string()));

        let img = registry
            .create(&CustomNodeContext {
                tag: "img",
                inner_text: "",
                container_style: &container_style,
                text_style: &text_style,
                measurer: Arc::clone(&measurer),
                images: &images,
                get_attr: &get_attr,
                write_back: None,
            })
            .unwrap();
        assert_eq!(img.role(), None);
    }

    #[test]
    fn registry_returns_none_for_unknown_tag() {
        let registry = ComponentRegistry::new();
        let attrs: HashMap<String, String> = HashMap::new();
        let container_style = ContainerStyle::default();
        let text_style = TextStyle::default();
        let images: HashMap<String, Image> = HashMap::new();
        let get_attr = |name: &str| attrs.get(name).cloned();
        let ctx = CustomNodeContext {
            tag: "video",
            inner_text: "",
            container_style: &container_style,
            text_style: &text_style,
            measurer: Arc::new(FallbackTextMeasurer),
            images: &images,
            get_attr: &get_attr,
            write_back: None,
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
