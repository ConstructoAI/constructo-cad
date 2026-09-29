// OpenCADStudio hatch pattern catalog — built from `assets/patterns/OpenCADStudio.pat`.
//
// Each `PatternEntry` wraps a parsed PAT pattern with:
//   - `gpu`       — `HatchPattern::Pattern(families)` for the shader
//   - `pat_lines` — exact PAT line definitions used for DXF export

use std::sync::OnceLock;

use crate::io::patterns::PatLineDef;
use crate::scene::model::hatch_model::{HatchPattern, PatFamily};
use acadrust::entities::{HatchPattern as DxfPattern, HatchPatternLine};
use acadrust::types::Vector2;

// ── Public types ──────────────────────────────────────────────────────────

pub struct PatternEntry {
    pub name: String,
    pub description: String,
    /// GPU-ready pattern for the shader.
    pub gpu: HatchPattern,
    /// Exact PAT line families (used for DXF export).
    pub pat_lines: Vec<PatLineDef>,
}

// ── Catalog ───────────────────────────────────────────────────────────────

static CATALOG: OnceLock<Vec<PatternEntry>> = OnceLock::new();
static IMPERIAL_CATALOG: OnceLock<Vec<PatternEntry>> = OnceLock::new();

/// The bundled catalog, in millimetres (acadiso.pat).
pub fn catalog() -> &'static [PatternEntry] {
    CATALOG.get_or_init(|| build_catalog(false))
}

/// The catalog in one units system. The imperial one is acad.pat: every
/// length 25.4 times smaller (ANSI31 lines 1/8" apart instead of 3.175), the
/// ISO and JIS patterns unchanged, as both files give them in millimetres.
pub fn catalog_for(imperial: bool) -> &'static [PatternEntry] {
    if imperial {
        IMPERIAL_CATALOG.get_or_init(|| build_catalog(true))
    } else {
        catalog()
    }
}

/// A pattern of the metric catalog; for a new hatch use [`find_for`] or
/// [`find_in`], which follow the drawing's units.
pub fn find(name: &str) -> Option<&'static PatternEntry> {
    find_for(name, false)
}

/// A pattern of the catalog in one units system (see [`catalog_for`]).
pub fn find_for(name: &str, imperial: bool) -> Option<&'static PatternEntry> {
    catalog_for(imperial)
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(name))
}

/// A pattern as the drawing uses it: imperial for a drawing in inches or feet
/// (INSUNITS), metric otherwise — and metric for a drawing made in inches
/// before the catalogs followed the units, whose script compensates (see
/// `linetypes::document_uses_imperial_catalog`).
pub fn find_in(
    document: &acadrust::CadDocument,
    name: &str,
) -> Option<&'static PatternEntry> {
    find_for(name, crate::io::linetypes::document_uses_imperial_catalog(document))
}

// ── DXF export ────────────────────────────────────────────────────────────

pub fn build_dxf_pattern(entry: &PatternEntry) -> DxfPattern {
    let mut pat = DxfPattern::new(&entry.name);
    pat.description = entry.description.clone();
    for ln in &entry.pat_lines {
        let angle_rad = (ln.angle_deg as f64).to_radians();
        // The catalog stores the step (dx = shift along the line, dy =
        // perpendicular spacing) in the pattern LINE-LOCAL frame. The DWG
        // `HatchPatternLine.offset` is a WORLD-space vector — that is how real
        // files store it and how `family_from_stored_line` reads it back (it
        // inverse-rotates by the line angle). Emitting the raw local step here
        // made the reader recover a rotated, too-dense spacing (e.g. ANSI31 at
        // 45° collapsed 3.175 → 2.245). Rotate local → world by the line angle
        // so the offset is format-correct and round-trips to the exact spacing.
        let (ca, sa) = (angle_rad.cos(), angle_rad.sin());
        let (ldx, ldy) = (ln.dx as f64, ln.dy as f64);
        pat.lines.push(HatchPatternLine {
            angle: angle_rad,
            base_point: Vector2::new(ln.x0 as f64, ln.y0 as f64),
            offset: Vector2::new(ldx * ca - ldy * sa, ldx * sa + ldy * ca),
            dash_lengths: ln.dashes.iter().map(|&d| d as f64).collect(),
        });
    }
    pat
}

// ── Builder ───────────────────────────────────────────────────────────────

fn build_catalog(imperial: bool) -> Vec<PatternEntry> {
    let mut entries = vec![PatternEntry {
        name: "SOLID".into(),
        description: "Solid fill".into(),
        gpu: HatchPattern::Solid,
        pat_lines: vec![],
    }];

    for def in crate::io::patterns::catalog() {
        let lines: Vec<PatLineDef> =
            if imperial && !crate::io::linetypes::metric_in_both_catalogs(&def.name) {
                def.lines.iter().map(to_inches).collect()
            } else {
                def.lines.clone()
            };
        entries.push(PatternEntry {
            name: def.name.clone(),
            description: def.description.clone(),
            gpu: HatchPattern::Pattern(lines.iter().map(pat_line_to_family).collect()),
            pat_lines: lines,
        });
    }
    entries
}

/// A metric pattern line family in inches: origin, step and dashes divided by
/// 25.4 (computed in f64, so 3.175 gives exactly 0.125).
fn to_inches(line: &PatLineDef) -> PatLineDef {
    let inches = |value: f32| (f64::from(value) / crate::io::linetypes::MM_PER_INCH) as f32;
    PatLineDef {
        angle_deg: line.angle_deg,
        x0: inches(line.x0),
        y0: inches(line.y0),
        dx: inches(line.dx),
        dy: inches(line.dy),
        dashes: line.dashes.iter().map(|&dash| inches(dash)).collect(),
    }
}

fn pat_line_to_family(ln: &PatLineDef) -> PatFamily {
    PatFamily {
        angle_deg: ln.angle_deg,
        x0: ln.x0,
        y0: ln.y0,
        dx: ln.dx,
        dy: ln.dy,
        dashes: ln.dashes.clone(),
    }
}

#[cfg(test)]
mod units_tests {
    use super::{build_dxf_pattern, catalog_for, find_for, find_in};
    use crate::scene::model::hatch_model::HatchPattern;

    fn spacing(name: &str, imperial: bool) -> f64 {
        let entry = find_for(name, imperial).unwrap_or_else(|| panic!("{name}"));
        let line = &build_dxf_pattern(entry).lines[0];
        line.offset.x.hypot(line.offset.y)
    }

    /// acad.pat: ANSI31 lines 1/8" apart, ANSI37 the same crossed, AR-CONC
    /// and EARTH 25.4 times smaller than in acadiso; ISO and JIS unchanged.
    #[test]
    fn the_imperial_catalog_is_acad_pat() {
        assert!((spacing("ANSI31", true) - 0.125).abs() < 1e-7);
        assert!((spacing("ANSI31", false) - 3.175).abs() < 1e-5);
        assert!((spacing("ANSI37", true) - 0.125).abs() < 1e-7);
        let earth = find_for("EARTH", true).unwrap();
        assert!((earth.pat_lines[0].dx - 0.25).abs() < 1e-7);
        assert!((earth.pat_lines[0].dashes[0] - 0.25).abs() < 1e-7);
        let conc = find_for("AR-CONC", true).unwrap();
        assert!((conc.pat_lines[0].dashes[0] - 0.75).abs() < 1e-6, "{:?}", conc.pat_lines[0]);
        assert!((spacing("ISO02W100", true) - spacing("ISO02W100", false)).abs() < 1e-9);
        assert!((spacing("JIS_LC_20", true) - 20.0).abs() < 1e-5);
        let HatchPattern::Pattern(families) = &find_for("ANSI31", true).unwrap().gpu else {
            panic!("ANSI31 is a line pattern")
        };
        assert!((families[0].dy - 0.125).abs() < 1e-7, "the shader draws it too");
        assert!(matches!(find_for("SOLID", true).unwrap().gpu, HatchPattern::Solid));
        assert_eq!(catalog_for(true).len(), catalog_for(false).len());
    }

    #[test]
    fn a_drawing_picks_the_catalog_of_its_units() {
        let mut doc = acadrust::CadDocument::new();
        let metric = find_in(&doc, "ANSI31").unwrap();
        assert!((metric.pat_lines[0].dy - 3.175).abs() < 1e-5);
        doc.header.insertion_units = 1;
        let inches = find_in(&doc, "ansi31").unwrap();
        assert!((inches.pat_lines[0].dy - 0.125).abs() < 1e-7);
        crate::io::linetypes::populate_document(&mut doc);
        let inches = find_in(&doc, "ANSI31").unwrap();
        assert!((inches.pat_lines[0].dy - 0.125).abs() < 1e-7, "acad.lin HIDDEN");
    }

    /// A drawing made in inches before the fix holds the metric HIDDEN: its
    /// script scales its hatches by 1/25.4, so its new hatches stay metric.
    #[test]
    fn a_drawing_made_before_the_fix_keeps_the_metric_patterns() {
        let mut doc = acadrust::CadDocument::new();
        crate::io::linetypes::populate_document(&mut doc);
        doc.header.insertion_units = 1;
        let legacy = find_in(&doc, "ANSI31").unwrap();
        assert!((legacy.pat_lines[0].dy - 3.175).abs() < 1e-5);
    }
}
