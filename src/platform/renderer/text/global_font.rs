use std::sync::{LazyLock, Mutex};

use orinium_text::FontSystem;

static GLOBAL_FONT_SYSTEM: LazyLock<Mutex<FontSystem>> = LazyLock::new(|| {
    let font_sys = if let Ok(p) = std::env::var("ORINIUM_FONT") {
        let source = orinium_text::fontdb::Source::File(p.into());
        FontSystem::new_with_fonts(vec![source])
    } else {
        FontSystem::new()
    };
    Mutex::new(font_sys)
});

pub fn with_global_font_system<T>(f: impl FnOnce(&mut FontSystem) -> T) -> T {
    let mut fs = GLOBAL_FONT_SYSTEM
        .lock()
        .expect("GLOBAL_FONT_SYSTEM lock failed");
    f(&mut *fs)
}

pub fn global_font_system_ready() -> bool {
    GLOBAL_FONT_SYSTEM
        .lock()
        .map(|fs| fs.db.len() > 0)
        .unwrap_or(false)
}

#[allow(dead_code)]
pub fn load_global_font_data(data: Vec<u8>) -> Vec<orinium_text::FontKey> {
    let mut fs = GLOBAL_FONT_SYSTEM
        .lock()
        .expect("GLOBAL_FONT_SYSTEM lock failed");
    fs.load_font_data(data)
}

pub fn load_global_font_path(path: &str) -> Option<Vec<orinium_text::FontKey>> {
    let bytes = std::fs::read(path).ok()?;
    let mut fs = GLOBAL_FONT_SYSTEM
        .lock()
        .expect("GLOBAL_FONT_SYSTEM lock failed");
    Some(fs.load_font_data(bytes))
}
