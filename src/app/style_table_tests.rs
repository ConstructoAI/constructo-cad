//! Styles and table entries made without a screen, as the ERP makes them, and
//! what the DWG and the DXF keep of them (T4 D-1 and D-3, T3 M1, M2 and m3).

use super::OpenCADStudio;
use acadrust::tables::DimStyle;
use acadrust::types::Handle;
use acadrust::EntityType;
use serde_json::{json, Value};

fn fresh_app() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], true);
    app
}

fn run_ok(app: &mut OpenCADStudio, cmd: &str) -> Value {
    let response = app.automation_op(&json!({ "op": "run", "cmd": cmd }).to_string());
    assert_eq!(response["ok"], true, "{cmd}: {response}");
    response
}

/// One request on the control channel, as `control::tests::request` does.
fn control(app: &mut OpenCADStudio, mut req: Value) -> Value {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let state = app.control_state();
    req["protocol"] = json!(1);
    req["document_id"] = state["document_id"].clone();
    req["revision"] = state["revision"].clone();
    req["client_id"] = json!("test");
    req["request_id"] = json!(format!("styles-{}", SERIAL.fetch_add(1, Ordering::Relaxed)));
    let (response, task) = app.control_request(req.clone());
    app.drive_headless_task(task).unwrap();
    if matches!(response["status"].as_str(), Some("accepted" | "running")) {
        app.control_request(json!({"op":"operation","request_id":req["request_id"]}))
            .0
    } else {
        response
    }
}

fn set_style(app: &mut OpenCADStudio, collection: &str, name: &str, updates: Value) {
    let edited = control(
        app,
        json!({"op":"set_properties","collection":collection,"name":name,"updates":updates}),
    );
    assert_ne!(edited["ok"], false, "{collection} {name}: {edited}");
}

fn temp_file(stem: &str, extension: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("ocs-styles-{}-{stem}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir
        .join(format!("{stem}.{extension}"))
        .to_string_lossy()
        .replace('\\', "/");
    (dir, path)
}

fn save_and_reopen(app: &mut OpenCADStudio, stem: &str, extension: &str) -> OpenCADStudio {
    let (_, path) = temp_file(stem, extension);
    let saved = app.automation_op(&json!({ "op": "save", "path": path }).to_string());
    assert_eq!(saved["ok"], true, "{extension}: {saved}");
    let mut reopened = fresh_app();
    let opened = reopened.automation_op(&json!({ "op": "open", "path": path }).to_string());
    assert_eq!(opened["ok"], true, "{extension}: {opened}");
    reopened
}

fn dim_style(app: &OpenCADStudio, name: &str) -> DimStyle {
    app.tabs[app.active_tab]
        .scene
        .document
        .dim_styles
        .get(name)
        .unwrap_or_else(|| panic!("dimension style {name} missing"))
        .clone()
}

fn text_style_name(app: &OpenCADStudio, handle: Handle) -> Option<String> {
    app.tabs[app.active_tab]
        .scene
        .document
        .text_styles
        .iter()
        .find(|style| style.handle == handle)
        .map(|style| style.name.clone())
}

const DIMENSION_STYLES: [(&str, f64, &str); 3] =
    [("ARCH-8", 8.0, "ARCH-GRAS"), ("ARCH-24", 24.0, "ARCH"), ("ARCH-48", 48.0, "ARCH")];

/// T4 D-1 and T3 M1: three dimension styles and two text styles made in one
/// session, each dimension style given its format and font through the records
/// API (the ERP's `style_cote`), one made current. Each has its own handle, and
/// the DWG and the DXF keep all five, their settings and the current styles:
/// a dimension drawn after reopening takes the current style. Before, every
/// `DIMSTYLE NEW` got the handle 0: the edit of the second replaced the first,
/// the writers kept one, and $DIMSTYLE fell back to Standard.
#[test]
fn styles_made_in_one_session_survive_dwg_and_dxf() {
    let mut app = fresh_app();
    run_ok(&mut app, "INSUNITS 1");
    run_ok(&mut app, "STYLE NEW ARCH arial.ttf");
    run_ok(&mut app, "STYLE NEW ARCH-GRAS arialbd.ttf");
    for (name, scale, font) in DIMENSION_STYLES {
        run_ok(&mut app, &format!("DIMSTYLE NEW {name}"));
        run_ok(&mut app, &format!("DIMSTYLE SET {name} dimscale {scale}"));
        set_style(
            &mut app,
            "dim_styles",
            name,
            json!([
                {"path":"/dimlunit","value":4},
                {"path":"/dimzin","value":3},
                {"path":"/dimfrac","value":2},
                {"path":"/dimtxsty","value":font}
            ]),
        );
    }
    run_ok(&mut app, "CDIMSTY ARCH-24");
    run_ok(&mut app, "STYLE SET ARCH-GRAS");

    let doc = &app.tabs[app.active_tab].scene.document;
    let mut handles: Vec<Handle> = ["ARCH-8", "ARCH-24", "ARCH-48"]
        .iter()
        .map(|name| doc.dim_styles.get(name).unwrap().handle)
        .chain(["ARCH", "ARCH-GRAS"].iter().map(|name| doc.text_styles.get(name).unwrap().handle))
        .collect();
    assert!(handles.iter().all(|handle| !handle.is_null()), "{handles:?}");
    handles.sort_by_key(|handle| handle.value());
    handles.dedup();
    assert_eq!(handles.len(), 5, "five distinct handles");
    assert_eq!(doc.header.current_dimstyle_name, "ARCH-24");
    assert_eq!(doc.header.current_dimstyle_handle, dim_style(&app, "ARCH-24").handle);

    for extension in ["dwg", "dxf"] {
        let mut reopened = save_and_reopen(&mut app, &format!("five-{extension}"), extension);
        for (name, scale, font) in DIMENSION_STYLES {
            let style = dim_style(&reopened, name);
            assert!((style.dimscale - scale).abs() < 1e-9, "{extension} {name}: {}", style.dimscale);
            assert_eq!(style.dimlunit, 4, "{extension} {name}");
            assert_eq!(style.dimzin & 3, 3, "{extension} {name}");
            assert_eq!(style.dimfrac, 2, "{extension} {name}");
            assert_eq!(
                text_style_name(&reopened, style.dimtxsty_handle).as_deref(),
                Some(font),
                "{extension} {name}: DIMTXSTY"
            );
        }
        let document = &reopened.tabs[reopened.active_tab].scene.document;
        for (name, font) in [("ARCH", "arial.ttf"), ("ARCH-GRAS", "arialbd.ttf")] {
            assert_eq!(
                document.text_styles.get(name).map(|style| style.font_file.as_str()),
                Some(font),
                "{extension} {name}"
            );
        }
        assert!(
            document.header.current_dimstyle_name.eq_ignore_ascii_case("ARCH-24"),
            "{extension}: $DIMSTYLE {}",
            document.header.current_dimstyle_name
        );
        assert!(
            document.header.current_text_style_name.eq_ignore_ascii_case("ARCH-GRAS"),
            "{extension}: $TEXTSTYLE {}",
            document.header.current_text_style_name
        );
        // The next script cotes with the current style, not Standard.
        run_ok(&mut reopened, "DIMLINEAR 0,0 240,0 H 120,-20");
        let drawn = reopened.tabs[reopened.active_tab]
            .scene
            .document
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Dimension(dimension) => Some(dimension.base().style_name.clone()),
                _ => None,
            })
            .last()
            .unwrap();
        assert!(drawn.eq_ignore_ascii_case("ARCH-24"), "{extension}: drawn in {drawn}");
    }
}

/// Two entries sharing the NULL handle (a drawing made before, or a plugin):
/// a records edit of the second leaves the first alone, and the save gives
/// both a handle, so both come back.
#[test]
fn entries_without_a_handle_stay_apart() {
    let mut app = fresh_app();
    let i = app.active_tab;
    for name in ["LEGACY-A", "LEGACY-B"] {
        let mut style = DimStyle::new(name);
        style.dimscale = 1.0;
        let _ = app.tabs[i].scene.document.dim_styles.add(style);
    }
    set_style(&mut app, "dim_styles", "LEGACY-B", json!([{"path":"/dimscale","value":96.0}]));
    assert!((dim_style(&app, "LEGACY-A").dimscale - 1.0).abs() < 1e-9, "A untouched");
    assert_eq!(dim_style(&app, "LEGACY-A").name, "LEGACY-A");
    assert!((dim_style(&app, "LEGACY-B").dimscale - 96.0).abs() < 1e-9);
    for extension in ["dwg", "dxf"] {
        let reopened = save_and_reopen(&mut app, &format!("legacy-{extension}"), extension);
        assert!((dim_style(&reopened, "LEGACY-A").dimscale - 1.0).abs() < 1e-9, "{extension}");
        assert!((dim_style(&reopened, "LEGACY-B").dimscale - 96.0).abs() < 1e-9, "{extension}");
    }
}

/// TABLESTYLE NEW and MLSTYLE NEW took `next_handle` without advancing it:
/// the next object made got the same handle.
#[test]
fn new_table_and_multiline_styles_take_their_own_handles() {
    let mut app = fresh_app();
    run_ok(&mut app, "TABLESTYLE NEW TABLEAU");
    run_ok(&mut app, "MLSTYLE NEW MURS");
    run_ok(&mut app, "LINE 0,0 10,0");
    let doc = &app.tabs[app.active_tab].scene.document;
    let style_handles: Vec<Handle> = doc
        .objects
        .values()
        .filter_map(|object| match object {
            acadrust::objects::ObjectType::TableStyle(style) if style.name == "TABLEAU" => {
                Some(style.handle)
            }
            acadrust::objects::ObjectType::MLineStyle(style) if style.name == "MURS" => {
                Some(style.handle)
            }
            _ => None,
        })
        .collect();
    assert_eq!(style_handles.len(), 2);
    let line = doc
        .entities()
        .find_map(|entity| match entity {
            EntityType::Line(line) => Some(line.common.handle),
            _ => None,
        })
        .unwrap();
    assert_ne!(style_handles[0], style_handles[1]);
    assert!(!style_handles.contains(&line), "{style_handles:?} and the line {line:?}");
}

/// T4 D-3: the reference's architectural tick, by name and without a screen.
/// `DIMSTYLE SET <style> dimblk _ARCHTICK` makes the standard `_ArchTick`
/// block (one polyline segment of width 0.15), points both ends at it and
/// clears DIMTSZ; the dimension picture draws it heavy (width 0.15 × DIMASZ ×
/// DIMSCALE) and runs the dimension line DIMDLE past the extension lines.
#[test]
fn the_architectural_tick_is_set_by_name_and_drawn_heavy() {
    let mut app = fresh_app();
    run_ok(&mut app, "INSUNITS 1");
    run_ok(&mut app, "DIMSTYLE NEW ARCH-8");
    for (property, value) in [
        ("dimscale", "8"),
        ("dimasz", "0.109375"),
        ("dimtsz", "0.109375"),
        ("dimdle", "0.0625"),
    ] {
        run_ok(&mut app, &format!("DIMSTYLE SET ARCH-8 {property} {value}"));
    }
    run_ok(&mut app, "DIMSTYLE SET ARCH-8 dimblk _ARCHTICK");
    let style = dim_style(&app, "ARCH-8");
    let doc = &app.tabs[app.active_tab].scene.document;
    let block = doc.block_records.get("_ArchTick").expect("_ArchTick made").clone();
    assert_eq!(style.dimblk, block.handle);
    assert_eq!((style.dimblk1, style.dimblk2), (block.handle, block.handle));
    assert!(!style.dimsah);
    assert_eq!(style.dimtsz, 0.0, "a tick size would hide the block");
    let stroke = block
        .entity_handles
        .iter()
        .find_map(|handle| match doc.get_entity(*handle) {
            Some(EntityType::LwPolyline(stroke)) => Some(stroke.clone()),
            _ => None,
        })
        .expect("the tick's polyline");
    assert!((stroke.constant_width - 0.15).abs() < 1e-12);
    assert_eq!(stroke.vertices.len(), 2);

    // An arrowhead this command does not make is refused, the style unchanged.
    let refused = app.automation_op(
        &json!({ "op": "run", "cmd": "DIMSTYLE SET ARCH-8 dimblk _DOTSMALL" }).to_string(),
    );
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(dim_style(&app, "ARCH-8").dimblk, block.handle);

    run_ok(&mut app, "CDIMSTY ARCH-8");
    run_ok(&mut app, "DIMLINEAR 0,0 100,0 H 50,20");
    for extension in ["dwg", "dxf"] {
        let reopened = save_and_reopen(&mut app, &format!("archtick-{extension}"), extension);
        let doc = &reopened.tabs[reopened.active_tab].scene.document;
        let style = dim_style(&reopened, "ARCH-8");
        let named = doc
            .block_records
            .iter()
            .find(|record| record.handle == style.dimblk)
            .map(|record| record.name.clone());
        assert_eq!(named.as_deref(), Some("_ArchTick"), "{extension}: DIMBLK");
        let dimension = doc
            .entities()
            .find_map(|entity| match entity {
                EntityType::Dimension(dimension) => Some(dimension.clone()),
                _ => None,
            })
            .unwrap();
        let picture = doc
            .block_records
            .get(&dimension.base().block_name)
            .expect("the dimension's picture");
        let mut widths = Vec::new();
        let mut longest: f64 = 0.0;
        for handle in &picture.entity_handles {
            match doc.get_entity(*handle) {
                Some(EntityType::LwPolyline(stroke)) => widths.push(stroke.constant_width),
                Some(EntityType::Line(line)) if (line.start.y - line.end.y).abs() < 1e-9 => {
                    longest = longest.max((line.end.x - line.start.x).abs());
                }
                _ => {}
            }
        }
        assert_eq!(widths.len(), 2, "{extension}: two heavy ticks, {widths:?}");
        for width in widths {
            assert!((width - 0.15 * 0.109375 * 8.0).abs() < 1e-9, "{extension}: width {width}");
        }
        assert!(
            (longest - (100.0 + 2.0 * 0.0625 * 8.0)).abs() < 1e-9,
            "{extension}: dimension line {longest}, DIMDLE past both ends"
        );
    }
}

/// T3 M2: an explicit transparency is written with method 2 in a DWG, as
/// AutoCAD and ODA write it (method 3 read back as ByBlock, hence opaque, in
/// ezdxf), and comes back at the same amount. The glass of the reference 3D
/// model is 0.65.
#[test]
fn an_explicit_transparency_keeps_its_method_and_amount() {
    let glass = acadrust::types::Transparency::from_percent(0.65);
    assert_eq!(glass.to_alpha_value() as u32, 0x0200_0059);
    let mut app = fresh_app();
    run_ok(&mut app, "LINE 0,0 10,0");
    let i = app.active_tab;
    for entity in app.tabs[i].scene.document.entities_mut() {
        if let EntityType::Line(line) = entity {
            line.common.transparency = glass;
        }
    }
    let reopened = save_and_reopen(&mut app, "verre", "dwg");
    let read = reopened.tabs[reopened.active_tab]
        .scene
        .document
        .entities()
        .find_map(|entity| match entity {
            EntityType::Line(line) => Some(line.common.transparency),
            _ => None,
        })
        .unwrap();
    assert_eq!(read, glass);
}

/// T3 m3: after a reopen the document also holds the dimensions' pictures;
/// the entities summary says what the model and the sheets hold apart.
#[test]
fn the_summary_counts_the_spaces_apart_from_the_blocks() {
    let mut app = fresh_app();
    run_ok(&mut app, "LINE 0,0 10,0");
    run_ok(&mut app, "DIMLINEAR 0,0 10,0 H 5,5");
    let mut reopened = save_and_reopen(&mut app, "inventaire", "dwg");
    let before = reopened.automation_op(r#"{"op":"entities"}"#);
    run_ok(&mut reopened, "LINE 0,10 10,10");
    let after = reopened.automation_op(r#"{"op":"entities"}"#);
    let count = |summary: &Value, key: &str| summary[key].as_u64().unwrap();
    assert_eq!(count(&after, "in_spaces"), count(&before, "in_spaces") + 1, "{before} / {after}");
    assert!(count(&before, "in_blocks") > 0, "the picture of the dimension: {before}");
    assert!(
        count(&before, "total") >= count(&before, "in_spaces") + count(&before, "in_blocks"),
        "{before}"
    );
    let lines_in_spaces = reopened.tabs[reopened.active_tab]
        .scene
        .document
        .entities()
        .filter(|entity| matches!(entity, EntityType::Line(_)))
        .count() as u64;
    assert!(
        lines_in_spaces > 2,
        "the document still holds the picture's lines too ({lines_in_spaces}): `total` is unchanged"
    );
}
