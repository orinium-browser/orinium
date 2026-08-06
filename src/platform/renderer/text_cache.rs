use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use smol_str::SmolStr;

use orinium_text::ShapedText;

#[derive(Clone)]
pub struct TextShapeCache {
    inner: std::sync::Arc<Mutex<HashMap<TextShapeCacheKey, ShapedText>>>,
}

impl TextShapeCache {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, text: &str, style: &orinium_text::TextStyle) -> Option<ShapedText> {
        let key = TextShapeCacheKey::new(text, style);

        self.inner
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())
    }

    pub fn insert(&self, text: &str, style: &orinium_text::TextStyle, shaped: ShapedText) {
        let key = TextShapeCacheKey::new(text, style);

        if let Ok(mut cache) = self.inner.lock() {
            cache.insert(key, shaped);
        }
    }
}

#[derive(Clone, Eq)]
struct TextShapeCacheKey {
    text: SmolStr,
    style_hash: u64,
}

impl TextShapeCacheKey {
    fn new(text: &str, style: &orinium_text::TextStyle) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_style(style, &mut hasher);

        Self {
            text: SmolStr::new(text),
            style_hash: hasher.finish(),
        }
    }
}

impl PartialEq for TextShapeCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.style_hash == other.style_hash
    }
}

impl Hash for TextShapeCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.style_hash.hash(state);
    }
}

fn hash_style(style: &orinium_text::TextStyle, hasher: &mut impl Hasher) {
    style.font_size.to_bits().hash(hasher);
    style.line_height.to_bits().hash(hasher);

    style.color.0.hash(hasher);
    style.color.1.hash(hasher);
    style.color.2.hash(hasher);
    style.color.3.hash(hasher);

    style.font_weight.0.hash(hasher);
    style.font_style.hash(hasher);

    style.bidi_mode.hash(hasher);
    style.variant.hash(hasher);

    for family in &style.font_families {
        family.hash(hasher);
    }

    for font in &style.exact_fonts {
        font.hash(hasher);
    }
}
