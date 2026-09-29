//! Viewport-specific layer overrides (VPLAYER colour, transparency, linetype,
//! lineweight) are read through the layer's own extension-dictionary pointer.
//!
//! Renderers ask for them once per entity, per viewport and per kind of
//! override, so the lookup must not grow with the drawing. Every reader (DWG,
//! DXF) and `ensure_extension_dictionary` record a layer's extension
//! dictionary; before this, a layer WITHOUT one -- most layers -- sent every
//! lookup through a scan of all the objects of the drawing, looking for a
//! dictionary that names the layer as its owner. On a real 44-sheet AutoCAD
//! set that scan alone cost about 15 s of a 70 s freeze of one sheet.
//!
//! The fixtures are written by the crate itself: no sample drawing is needed.

use std::io::Cursor;

use acadrust::entities::{EntityType, Viewport};
use acadrust::objects::{Dictionary, KnownXRecordKind, ObjectType, XRecord, XRecordValue};
use acadrust::tables::Layer;
use acadrust::{CadDocument, DwgReader, DwgWriter, DxfReader, DxfWriter, Handle};

const RED: i32 = 0x00FF_0000;

struct Fixture {
    doc: CadDocument,
    overridden_viewport: Handle,
    other_viewport: Handle,
}

/// A layer as the readers and the engines leave it: with its own handle
/// (`Table::add` does not allocate one, and a layer without a handle would
/// share the null owner of the root dictionary).
fn add_layer(doc: &mut CadDocument, name: &str) {
    let mut layer = Layer::new(name);
    layer.handle = doc.allocate_handle();
    doc.layers.add(layer).unwrap();
}

fn fixture() -> Fixture {
    let mut doc = CadDocument::new();
    add_layer(&mut doc, "OVERRIDDEN");
    add_layer(&mut doc, "PLAIN");
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
    let layer = doc.layers.get("OVERRIDDEN").unwrap().handle;
    assert!(doc.set_layer_viewport_override(
        layer,
        KnownXRecordKind::LayerViewportColorOverride,
        overridden_viewport,
        XRecordValue::Int32(RED),
    ));
    Fixture {
        doc,
        overridden_viewport,
        other_viewport,
    }
}

fn assert_overrides(doc: &CadDocument, overridden_viewport: Handle, format: &str) {
    let overridden = doc.layers.get("OVERRIDDEN").unwrap().handle;
    let plain = doc.layers.get("PLAIN").unwrap().handle;
    let colors = doc.layer_viewport_overrides(overridden, KnownXRecordKind::LayerViewportColorOverride);
    assert_eq!(colors.len(), 1, "{format}: {colors:?}");
    assert_eq!(colors[0].0, overridden_viewport, "{format}: viewport of the override");
    assert_eq!(colors[0].1.as_i32(), Some(RED), "{format}: value of the override");
    for kind in [
        KnownXRecordKind::LayerViewportAlphaOverride,
        KnownXRecordKind::LayerViewportLinetypeOverride,
        KnownXRecordKind::LayerViewportLineweightOverride,
    ] {
        assert!(
            doc.layer_viewport_overrides(overridden, kind).is_empty(),
            "{format}: {kind:?} was never set"
        );
    }
    for kind in [
        KnownXRecordKind::LayerViewportAlphaOverride,
        KnownXRecordKind::LayerViewportColorOverride,
        KnownXRecordKind::LayerViewportLinetypeOverride,
        KnownXRecordKind::LayerViewportLineweightOverride,
    ] {
        assert!(
            doc.layer_viewport_overrides(plain, kind).is_empty(),
            "{format}: the plain layer has no override of any kind ({kind:?})"
        );
    }
}

#[test]
fn a_layer_override_is_found_in_memory() {
    let f = fixture();
    assert_ne!(f.overridden_viewport, f.other_viewport);
    assert_overrides(&f.doc, f.overridden_viewport, "memory");
}

#[test]
fn a_layer_override_is_found_after_a_dwg_roundtrip() {
    let f = fixture();
    let loaded = DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&f.doc).unwrap()))
        .read()
        .unwrap();
    assert_overrides(&loaded, f.overridden_viewport, "dwg");
}

#[test]
fn a_layer_override_is_found_after_a_dxf_roundtrip() {
    let f = fixture();
    let loaded = DxfReader::from_reader(Cursor::new(DxfWriter::new(&f.doc).write_to_vec().unwrap()))
        .unwrap()
        .read()
        .unwrap();
    assert_overrides(&loaded, f.overridden_viewport, "dxf");
}

/// A dictionary that names the layer as its owner, while the layer does not
/// point to it, is not the layer's extension dictionary -- the format resolves
/// an extension dictionary from the owner's side. Finding it would require
/// walking every object of the drawing for every lookup of a plain layer,
/// which is the cost this lookup must not pay.
#[test]
fn a_dictionary_the_layer_does_not_point_to_is_not_its_extension_dictionary() {
    let mut f = fixture();
    let plain = f.doc.layers.get("PLAIN").unwrap().handle;

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
    // The layer that does point to its dictionary is unaffected.
    assert_overrides(&f.doc, f.overridden_viewport, "memory, with a stray dictionary");
}
