//! Font loading for text rendering.
//!
//! To keep the engine deterministic, this only loads fonts the op names
//! **explicitly**: a concrete file path, or (when none is given, or the path is
//! unreadable) the embedded default — Cascadia Code (SIL OFL 1.1, see
//! `assets/CascadiaCode-LICENSE.txt`). Resolving font *family names* to files is
//! non-deterministic (depends on installed fonts) and lives in the ports, not
//! here. Loaded fonts are cached by path so repeated text ops stay cheap.

use ab_glyph::FontVec;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

const DEFAULT_FONT: &[u8] = include_bytes!("../assets/CascadiaCode.ttf");

static CACHE: LazyLock<Mutex<HashMap<String, Arc<FontVec>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Load a font by explicit path, or the embedded default. An unreadable or
/// unparseable path falls back to the default (rendering never hard-fails).
pub fn load(path: Option<&str>) -> Arc<FontVec> {
    let key = path.unwrap_or("");
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = cache.get(key) {
            return Arc::clone(f);
        }
    }
    let font = path
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| FontVec::try_from_vec(b).ok())
        .unwrap_or_else(default_font);
    let arc = Arc::new(font);
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), Arc::clone(&arc));
    arc
}

fn default_font() -> FontVec {
    FontVec::try_from_vec(DEFAULT_FONT.to_vec()).expect("embedded default font must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_loads_and_caches() {
        let a = load(None);
        let b = load(None);
        assert!(Arc::ptr_eq(&a, &b), "default is cached");
    }

    #[test]
    fn bad_path_falls_back_to_default() {
        let f = load(Some("C:/definitely/not/a/font.ttf"));
        // still a usable font (the embedded default)
        use ab_glyph::Font;
        assert!(f.glyph_id('A').0 > 0, "fallback font has an 'A' glyph");
    }
}
