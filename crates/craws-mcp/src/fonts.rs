//! Font-name resolution for the port. The engine only loads explicit font
//! paths (to stay deterministic); here we turn a user's font *name* — or path —
//! into a concrete file path using the system font database.

use std::path::Path;
use std::sync::LazyLock;

static DB: LazyLock<fontdb::Database> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
});

/// Resolve a font spec to an explicit file path for the engine:
/// - an existing file path is used as-is;
/// - otherwise it's treated as a family name and looked up in system fonts;
/// - `None` or an unresolved name → `None` (engine uses its embedded default).
pub fn resolve_font_path(spec: Option<&str>) -> Option<String> {
    let spec = spec?.trim();
    if spec.is_empty() {
        return None;
    }
    if Path::new(spec).is_file() {
        return Some(spec.to_string());
    }
    let families = [fontdb::Family::Name(spec)];
    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    };
    let id = DB.query(&query)?;
    match &DB.face(id)?.source {
        fontdb::Source::File(p) => Some(p.to_string_lossy().into_owned()),
        _ => None, // binary/in-memory face → no path; fall back to default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_font_resolves_to_none() {
        assert!(resolve_font_path(None).is_none());
        assert!(resolve_font_path(Some("   ")).is_none());
        assert!(resolve_font_path(Some("Totally Not A Real Font 12345")).is_none());
    }

    #[test]
    fn explicit_existing_path_passes_through() {
        // the vendored engine font is a stable, present file to probe with
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../craws-engine/assets/CascadiaCode.ttf");
        if Path::new(p).is_file() {
            assert_eq!(resolve_font_path(Some(p)).as_deref(), Some(p));
        }
    }
}
