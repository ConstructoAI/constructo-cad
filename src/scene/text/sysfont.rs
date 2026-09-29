// System TrueType/OpenType font discovery.
//
// Wraps a `fontdb` database loaded with the user's installed system fonts so
// the rest of the app can (a) list available font families for the text-style
// picker and (b) borrow a face's raw bytes to extract glyph outlines (see the
// TTF glyph engine). LFF stroke fonts stay separate — this is purely the
// TrueType side of the renderer.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

struct SysFonts {
    db: fontdb::Database,
    /// Sorted, de-duplicated family names for the picker.
    families: Vec<String>,
}

static FONTS: OnceLock<SysFonts> = OnceLock::new();

fn fonts() -> &'static SysFonts {
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // The browser offers no system fonts: without these, every text style
        // naming arial.ttf / arialbd.ttf fell back to the LFF stroke font,
        // wider than Arial (table texts overflowed their cells).
        #[cfg(target_arch = "wasm32")]
        for face in embedded_faces() {
            db.push_face_info(face);
        }

        let mut families: Vec<String> = db
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .collect();
        families.sort_by_key(|n| n.to_lowercase());
        families.dedup();

        SysFonts { db, families }
    })
}

/// All installed system font families, sorted case-insensitively, de-duped.
pub fn families() -> &'static [String] {
    &fonts().families
}

/// Resolve a requested family name to the canonical installed system family name (with exact case).
///
/// Memoised process-wide: `resolve_font` calls this once per word on the MTEXT
/// measure hot path for inline-`\f` TTF runs, and `Face::resolve` re-runs it
/// immediately after — the underlying fontdb query plus linear family scans are
/// not free. The cache keys on the raw request string; results are stable for
/// the process lifetime (the font DB is loaded once via `OnceLock`).
pub fn canonical_family_name(family: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(family) {
        return hit.clone();
    }
    let resolved = canonical_family_name_uncached(family);
    cache
        .lock()
        .unwrap()
        .insert(family.to_string(), resolved.clone());
    resolved
}

fn canonical_family_name_uncached(family: &str) -> Option<String> {
    let db = &fonts().db;
    
    // 1. Try exact match first
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        ..Default::default()
    };
    if db.query(&query).is_some() {
        if let Some(canonical) = fonts().families.iter().find(|&f| f.eq_ignore_ascii_case(family)) {
            return Some(canonical.clone());
        }
        return Some(family.to_string());
    }
    
    // 2. Try case-insensitive match on the families we have
    if let Some(matched) = fonts().families.iter().find(|&f| f.eq_ignore_ascii_case(family)) {
        return Some(matched.clone());
    }
    
    // 3. Match common prefixes / variations
    let family_lower = family.to_lowercase();
    // Web: Arial and Arial Bold resolve to the embedded Liberation Sans faces
    // (metric-compatible). Before the prefix match below, which would take
    // "arialbd" for the regular face.
    #[cfg(target_arch = "wasm32")]
    if let Some(substitute) = arial_substitute(&family_lower) {
        if fonts().families.iter().any(|f| f == substitute) {
            return Some(substitute.to_string());
        }
    }
    let alias = match family_lower.as_str() {
        "arialn" => Some("Arial Narrow"),
        "gothic" => Some("Century Gothic"),
        "times" => Some("Times New Roman"),
        "cour" => Some("Courier New"),
        _ => None,
    };
    
    if let Some(alias_name) = alias {
        if let Some(matched) = fonts().families.iter().find(|&f| f.eq_ignore_ascii_case(alias_name)) {
            return Some(matched.clone());
        }
    }
    
    // 4. Try matching prefix/subset case-insensitively. Require at least 3
    //    chars so a 1–2 letter request can't grab an arbitrary family by the
    //    first iteration order. Iterating sorted keeps the pick deterministic.
    if family_lower.len() >= 3 {
        let mut candidates: Vec<&String> = fonts()
            .families
            .iter()
            .filter(|&f| {
                let f_low = f.to_lowercase();
                f_low.starts_with(&family_lower) || family_lower.starts_with(&f_low)
            })
            .collect();
        candidates.sort();
        if let Some(matched) = candidates.first() {
            return Some((*matched).clone());
        }
    }

    None
}

/// Resolve a family name to a concrete face id (regular weight/style).
fn face_id(family: &str) -> Option<fontdb::ID> {
    let db = &fonts().db;
    let canonical = canonical_family_name(family)?;
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(&canonical)],
        ..Default::default()
    };
    db.query(&query)
}

/// Borrow the raw face bytes for `family` and run `f` over them. The byte slice
/// is only valid inside the closure, so callers extract everything they need
/// (e.g. flattened glyph outlines) before returning. `index` is the face index
/// within a TrueType collection. Returns `None` if the family is unknown.
pub fn with_face_data<T>(family: &str, f: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
    let id = face_id(family)?;
    fonts().db.with_face_data(id, f)
}

/// Whether `family` matches an installed system font (case-insensitive via
/// fontdb's own matching).
pub fn has_family(family: &str) -> bool {
    face_id(family).is_some()
}


// ── Fonts embedded for the web build ────────────────────────────────────────
//
// Liberation Sans 2.1.5 (SIL OFL 1.1, `assets/fonts/liberation/`), the free
// metric-compatible substitute for Arial: same advance widths glyph for glyph.
// Shipped UNMODIFIED ("Liberation" is a Reserved Font Name): the bold face is
// only registered under its own family name, so the family-only lookups of this
// module can reach it, and `cap_height_override` gives both faces Arial's cap
// height (Liberation's is 4 % shorter, and a TrueType text is scaled so its
// capitals are the text height: without it every text came out 4 % larger).

/// Family the embedded regular face answers to.
pub const ARIAL_SUBSTITUTE: &str = "Liberation Sans";
/// Family the embedded bold face answers to.
pub const ARIAL_BOLD_SUBSTITUTE: &str = "Liberation Sans Bold";

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
static LIBERATION_SANS_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/liberation/LiberationSans-Regular.ttf");
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
static LIBERATION_SANS_BOLD: &[u8] =
    include_bytes!("../../../assets/fonts/liberation/LiberationSans-Bold.ttf");

/// Arial's OS/2 cap height (units per em 2048, like Liberation Sans).
const ARIAL_CAP_HEIGHT: f32 = 1467.0;
const ARIAL_BOLD_CAP_HEIGHT: f32 = 1466.0;

/// The faces the web build adds to its (otherwise empty) font database.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn embedded_faces() -> Vec<fontdb::FaceInfo> {
    let face = |bytes: &'static [u8], family: &str, post_script: &str, weight| fontdb::FaceInfo {
        id: fontdb::ID::dummy(),
        source: fontdb::Source::Binary(std::sync::Arc::new(bytes)),
        index: 0,
        families: vec![(family.to_string(), fontdb::Language::English_UnitedStates)],
        post_script_name: post_script.to_string(),
        style: fontdb::Style::Normal,
        weight,
        stretch: fontdb::Stretch::Normal,
        monospaced: false,
    };
    vec![
        face(
            LIBERATION_SANS_REGULAR,
            ARIAL_SUBSTITUTE,
            "LiberationSans",
            fontdb::Weight::NORMAL,
        ),
        face(
            LIBERATION_SANS_BOLD,
            ARIAL_BOLD_SUBSTITUTE,
            "LiberationSans-Bold",
            fontdb::Weight::BOLD,
        ),
    ]
}

/// The embedded family standing in for a requested Arial name (lower case):
/// the style's font file (`arial.ttf` / `arialbd.ttf`, which the text-style
/// resolver hands over as `arial` / `arialbd`), a family name, or a PostScript
/// name. Other Arial families (Narrow, Black, italic) are not covered.
pub fn arial_substitute(requested_lower: &str) -> Option<&'static str> {
    let name = requested_lower.trim();
    let name = name.strip_suffix(".ttf").unwrap_or(name);
    match name {
        "arial" | "arialmt" | "arial regular" => Some(ARIAL_SUBSTITUTE),
        "arialbd" | "arial bold" | "arial-boldmt" | "arial-bold" => Some(ARIAL_BOLD_SUBSTITUTE),
        _ => None,
    }
}

/// The bold face of `family` when it is registered as a family of its own —
/// only the embedded Arial substitute (an MTEXT `\f…|b1;` run).
pub fn bold_variant(family: &str) -> Option<&'static str> {
    (family == ARIAL_SUBSTITUTE && has_family(ARIAL_BOLD_SUBSTITUTE)).then_some(ARIAL_BOLD_SUBSTITUTE)
}

/// Cap height (font units) the layout must use for `family` instead of the
/// face's own: Arial's, for the faces standing in for it.
pub fn cap_height_override(family: &str) -> Option<f32> {
    match family {
        ARIAL_SUBSTITUTE => Some(ARIAL_CAP_HEIGHT),
        ARIAL_BOLD_SUBSTITUTE => Some(ARIAL_BOLD_CAP_HEIGHT),
        _ => None,
    }
}

/// Whether `family` is one of the embedded faces (resolved by this module and
/// not by the web build's script fonts).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn is_embedded_family(family: &str) -> bool {
    matches!(family, ARIAL_SUBSTITUTE | ARIAL_BOLD_SUBSTITUTE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded_db() -> fontdb::Database {
        let mut db = fontdb::Database::new();
        for face in embedded_faces() {
            db.push_face_info(face);
        }
        db
    }

    fn query(db: &fontdb::Database, family: &str) -> Option<fontdb::ID> {
        db.query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            ..Default::default()
        })
    }

    #[test]
    fn arial_names_map_to_the_embedded_faces() {
        for name in ["arial", "arial.ttf", "arialmt", "arial regular"] {
            assert_eq!(arial_substitute(name), Some(ARIAL_SUBSTITUTE), "{name}");
        }
        for name in ["arialbd", "arialbd.ttf", "arial bold", "arial-boldmt"] {
            assert_eq!(arial_substitute(name), Some(ARIAL_BOLD_SUBSTITUTE), "{name}");
        }
        for name in ["arialn", "arial black", "ariali", "times", "txt", ""] {
            assert_eq!(arial_substitute(name), None, "{name}");
        }
    }

    // A family-only query (what `face_id` does) reaches each embedded face by
    // its own family name, with the right weight.
    #[test]
    fn each_embedded_face_answers_to_its_family() {
        let db = embedded_db();
        for (family, weight) in [
            (ARIAL_SUBSTITUTE, fontdb::Weight::NORMAL),
            (ARIAL_BOLD_SUBSTITUTE, fontdb::Weight::BOLD),
        ] {
            let id = query(&db, family).unwrap_or_else(|| panic!("{family} not found"));
            let info = db.face(id).expect("face");
            assert_eq!(info.weight, weight, "{family}");
            assert_eq!(info.families[0].0, family);
        }
    }

    // The files are the metric-compatible substitute: Arial's advance widths,
    // units per em and weights; only the cap height differs (overridden).
    #[test]
    fn embedded_faces_have_arial_widths() {
        let db = embedded_db();
        // (char, Arial advance, Arial Bold advance), units per em 2048.
        let arial = [('W', 1933, 1933), (' ', 569, 569), ('1', 1139, 1139), ('x', 1024, 1139)];
        for (family, bold) in [(ARIAL_SUBSTITUTE, false), (ARIAL_BOLD_SUBSTITUTE, true)] {
            let id = query(&db, family).expect("face");
            db.with_face_data(id, |data, index| {
                let face = ttf_parser::Face::parse(data, index).expect("parses");
                assert_eq!(face.units_per_em(), 2048, "{family}");
                assert_eq!(face.is_bold(), bold, "{family}");
                for (ch, regular, bold_width) in arial {
                    let gid = face.glyph_index(ch).expect("glyph");
                    let advance = face.glyph_hor_advance(gid).expect("advance");
                    assert_eq!(advance, if bold { bold_width } else { regular }, "{family} {ch:?}");
                }
                assert_eq!(face.capital_height(), Some(1409), "{family}: override needed");
            })
            .expect("face data");
        }
        assert_eq!(cap_height_override(ARIAL_SUBSTITUTE), Some(1467.0));
        assert_eq!(cap_height_override(ARIAL_BOLD_SUBSTITUTE), Some(1466.0));
        assert_eq!(cap_height_override("Arial"), None);
        assert!(is_embedded_family(ARIAL_SUBSTITUTE) && is_embedded_family(ARIAL_BOLD_SUBSTITUTE));
        assert!(!is_embedded_family("Noto Sans"));
    }
}
