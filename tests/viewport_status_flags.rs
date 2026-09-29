// Paper-space viewports written by something other than AutoCAD must show
// their model space. ezdxf (and the ODA File Converter run on its DXF) leave
// status bit 0x8000 clear — the reference calls it "currently always enabled",
// it carries no meaning — and acadrust took its absence for "off": every
// layout of such a drawing opened as empty viewport frames around the title
// block (CAD-000020 in the Constructo ERP viewer: 29 viewports flagged 0x4000).
// See `tests/fixtures/viewports/README.md` for the fixture.

use acadrust::entities::ViewportStatusFlags;
use acadrust::EntityType;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/viewports/status-flags-ezdxf-oda.dwg"
);

/// (view center x, view center y) → (group 90 written, expected locked).
const CONTENT: [((f64, f64), i32, bool); 12] = [
    ((1000.0, 0.0), 0x04000, true),
    ((1200.0, 0.0), 0x0C000, true),
    ((1400.0, 0.0), 0x08000, false),
    ((1600.0, 0.0), 0x00000, false),
    ((1800.0, 0.0), 0x28000, false),
    ((2000.0, 0.0), 0x20000, false),
    ((0.0, 0.0), 0x0C000, true),
    ((216000.0, 0.0), 0x0C000, true),
    ((0.0, -528000.0), 0x0C000, true),
    ((216000.0, -528000.0), 0x0C000, true),
    ((217200.0, -528000.0), 0x08000, false),
    ((218400.0, -528000.0), 0x0C000, true),
];

#[test]
fn viewports_written_by_ezdxf_and_oda_load_on() {
    let bytes = std::fs::read(FIXTURE).expect("fixture");
    let doc = OpenCADStudio::io::load_bytes("status-flags-ezdxf-oda.dwg", bytes).expect("load");
    let viewports: Vec<_> = doc
        .entities()
        .filter_map(|e| match e {
            EntityType::Viewport(vp) => Some(vp),
            _ => None,
        })
        .collect();
    assert_eq!(viewports.len(), 14, "12 content viewports and 2 overall ones");

    // acadrust 7ea4247 decodes `is_on` from 0x8000 and drops 0x20000, the bit
    // that turns a viewport off: until it reads that bit, a switched-off
    // viewport cannot be told apart and loads on (as it always did here).
    let off_bit_dropped = !ViewportStatusFlags::from_bits(0x4000).is_on;

    for ((x, y), bits, locked) in CONTENT {
        let vp = viewports
            .iter()
            .find(|vp| {
                (vp.width - 10.0).abs() < 1e-6
                    && (vp.view_center.x - x).abs() < 1e-6
                    && (vp.view_center.y - y).abs() < 1e-6
            })
            .unwrap_or_else(|| panic!("viewport looking at ({x}, {y}) missing"));
        let on = bits & 0x20000 == 0 || off_bit_dropped;
        assert_eq!(
            (vp.status.is_on, vp.status.locked),
            (on, locked),
            "group 90 = {bits:#07x}, view center ({x}, {y})"
        );
    }
    for vp in viewports.iter().filter(|vp| (vp.width - 39.6).abs() < 1e-6) {
        assert!(vp.status.is_on, "overall viewport of a layout");
    }
}
