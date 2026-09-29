//! Viewport-specific layer overrides (VPLAYER colour) reach the render style
//! of an entity, and only in their own viewport.
//!
//! The render asks acadrust for them once per entity, per viewport and per kind
//! of override. acadrust used to find a layer's extension dictionary, for a
//! layer that has none (most layers), by scanning every object of the drawing:
//! about 15 s of the 70 s freeze of sheet A300 of the real AutoCAD set C22-025.
//! The patched acadrust (`deps/cadcodec-7ea4247-constructo`) reads the layer's
//! own pointer instead. These tests pin the behaviour the render relies on, in
//! memory and after a DWG and a DXF round trip, and show that a dictionary the
//! layer does not point to is not consulted (that lookup was the scan).

use std::io::Cursor;

use acadrust::entities::{EntityType, Line, Viewport};
use acadrust::objects::{Dictionary, KnownXRecordKind, ObjectType, XRecord, XRecordValue};
use acadrust::tables::Layer;
use acadrust::types::{Color as AcadColor, Vector3};
use acadrust::{CadDocument, DwgReader, DwgWriter, DxfReader, DxfWriter, Handle};

use super::render::render_style_for_viewport;

const RED: i32 = 0x00FF_0000;
const GREEN: [f32; 3] = [0.0, 1.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

struct Fixture {
    doc: CadDocument,
    on_overridden_layer: Handle,
    on_plain_layer: Handle,
    overridden_viewport: Handle,
    other_viewport: Handle,
}

fn line_on(doc: &mut CadDocument, layer: &str, y: f64) -> Handle {
    let mut line = Line::from_points(Vector3::new(0.0, y, 0.0), Vector3::new(10.0, y, 0.0));
    line.common.layer = layer.to_string();
    line.common.color = AcadColor::ByLayer;
    doc.add_entity(EntityType::Line(line)).unwrap()
}

/// A layer as a reader or the LAYER command leaves it: with its own handle
/// (`Table::add` does not allocate one).
fn add_layer(doc: &mut CadDocument, name: &str, color: AcadColor) {
    let mut layer = Layer::new(name);
    layer.handle = doc.allocate_handle();
    layer.color = color;
    doc.layers.add(layer).unwrap();
}

fn fixture() -> Fixture {
    let mut doc = CadDocument::new();
    add_layer(&mut doc, "PLANCHER", AcadColor::Index(3));
    add_layer(&mut doc, "MURS", AcadColor::Index(5));
    let on_overridden_layer = line_on(&mut doc, "PLANCHER", 0.0);
    let on_plain_layer = line_on(&mut doc, "MURS", 5.0);
    let mut first = Viewport::new();
    first.id = 2;
    let overridden_viewport = doc
        .add_paper_space_entity(EntityType::Viewport(first))
        .unwrap();
    let mut second = Viewport::new();
    second.id = 3;
    let other_viewport = doc
        .add_paper_space_entity(EntityType::Viewport(second))
        .unwrap();
    let layer = doc.layers.get("PLANCHER").unwrap().handle;
    assert!(doc.set_layer_viewport_override(
        layer,
        KnownXRecordKind::LayerViewportColorOverride,
        overridden_viewport,
        XRecordValue::Int32(RED),
    ));
    Fixture {
        doc,
        on_overridden_layer,
        on_plain_layer,
        overridden_viewport,
        other_viewport,
    }
}

fn rgb(doc: &CadDocument, entity: Handle, viewport: Option<Handle>) -> [f32; 3] {
    let entity = doc.get_entity(entity).expect("the line");
    let ([r, g, b, _], ..) = render_style_for_viewport(doc, entity, viewport);
    [r, g, b]
}

fn assert_styles(doc: &CadDocument, f: &Fixture, format: &str) {
    let red = [1.0, 0.0, 0.0];
    assert_eq!(
        rgb(doc, f.on_overridden_layer, Some(f.overridden_viewport)),
        red,
        "{format}: the override of its own viewport"
    );
    assert_eq!(
        rgb(doc, f.on_overridden_layer, Some(f.other_viewport)),
        GREEN,
        "{format}: another viewport keeps the layer colour"
    );
    assert_eq!(
        rgb(doc, f.on_overridden_layer, None),
        GREEN,
        "{format}: model space keeps the layer colour"
    );
    for viewport in [Some(f.overridden_viewport), Some(f.other_viewport), None] {
        assert_eq!(
            rgb(doc, f.on_plain_layer, viewport),
            BLUE,
            "{format}: a layer without override, viewport {viewport:?}"
        );
    }
}

#[test]
fn a_viewport_layer_override_colours_only_its_own_viewport() {
    let f = fixture();
    assert_styles(&f.doc, &f, "memory");
}

#[test]
fn a_viewport_layer_override_survives_a_dwg_roundtrip() {
    let f = fixture();
    let loaded = DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&f.doc).unwrap()))
        .read()
        .unwrap();
    assert_styles(&loaded, &f, "dwg");
}

#[test]
fn a_viewport_layer_override_survives_a_dxf_roundtrip() {
    let f = fixture();
    let loaded = DxfReader::from_reader(Cursor::new(DxfWriter::new(&f.doc).write_to_vec().unwrap()))
        .unwrap()
        .read()
        .unwrap();
    assert_styles(&loaded, &f, "dxf");
}

/// A dictionary that names a layer as its owner, while the layer does not point
/// to it, is not the layer's extension dictionary. Finding it meant walking
/// every object of the drawing for every entity of a plain layer in every
/// viewport: the render must not see it.
#[test]
fn a_dictionary_the_layer_does_not_point_to_is_ignored() {
    let mut f = fixture();
    let plain = f.doc.layers.get("MURS").unwrap().handle;
    let record_handle = f.doc.allocate_handle();
    let dictionary_handle = f.doc.allocate_handle();
    let mut record = XRecord::named("ADSK_XREC_LAYER_COLOR_OVR");
    record.handle = record_handle;
    record.owner = dictionary_handle;
    record.set_layer_viewport_override(
        "ADSK_LYR_COLOR_OVERRIDE",
        420,
        f.other_viewport,
        XRecordValue::Int32(RED),
    );
    let mut stray = Dictionary::new();
    stray.handle = dictionary_handle;
    stray.owner = plain;
    stray.add_entry("ADSK_XREC_LAYER_COLOR_OVR", record_handle);
    f.doc.objects.insert(record_handle, ObjectType::XRecord(record));
    f.doc.objects.insert(dictionary_handle, ObjectType::Dictionary(stray));

    assert!(f
        .doc
        .layer_viewport_overrides(plain, KnownXRecordKind::LayerViewportColorOverride)
        .is_empty());
    assert_eq!(rgb(&f.doc, f.on_plain_layer, Some(f.other_viewport)), BLUE);
    assert_styles(&f.doc, &f, "memory, with a stray dictionary");
}
