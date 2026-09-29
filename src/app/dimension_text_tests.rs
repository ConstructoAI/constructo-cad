//! Where the text of a linear or aligned dimension lands when nobody placed it
//! by hand: where its style puts it — DIMTAD, DIMGAP × DIMSCALE and DIMTXT ×
//! DIMSCALE — at creation, after an override that changes the text, and after
//! the style itself is edited, as the reference application regenerates it.
//!
//! The creation commands used to store a fixed 0.15-unit offset, and a stored
//! point wins over the style: on a 1:8 detail the text sat 0.019" of paper
//! off the dimension line instead of 0.086" (half the text plus the gap).

use super::OpenCADStudio;
use crate::entities::dim_override;
use acadrust::entities::Dimension;
use acadrust::types::{Handle, Vector3};
use acadrust::EntityType;
use serde_json::{json, Value};

/// ARCH-8 of the reference sheets: 7/64" text and arrows, a 1/32" gap.
const TEXT: f64 = 0.109375;
const GAP: f64 = 0.03125;

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

/// A drawing in inches with the reference dimension style at `scale`, one
/// property per line as the ERP's scripts send them.
fn arch_style(app: &mut OpenCADStudio, scale: f64) {
    run_ok(app, "INSUNITS 1");
    for (property, value) in [
        ("dimscale", scale),
        ("dimtxt", TEXT),
        ("dimasz", TEXT),
        ("dimgap", GAP),
    ] {
        run_ok(app, &format!("DIMSTYLE SET Standard {property} {value}"));
    }
}

fn dimensions(app: &OpenCADStudio) -> Vec<(Handle, Dimension)> {
    app.tabs[app.active_tab]
        .scene
        .document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Dimension(dimension) => {
                Some((entity.common().handle, dimension.clone()))
            }
            _ => None,
        })
        .collect()
}

fn only_dimension(app: &OpenCADStudio) -> (Handle, Dimension) {
    let mut all = dimensions(app);
    assert_eq!(all.len(), 1, "one dimension expected");
    all.remove(0)
}

/// Distance from the text centre to the dimension line, on the side the text
/// reads "up" (above a horizontal line, left of a vertical one).
fn lift(dimension: &Dimension) -> f64 {
    let text = dimension.base().text_middle_point;
    let (on_line, mut ax, mut ay) = match dimension {
        Dimension::Linear(d) => (d.definition_point, d.rotation.cos(), d.rotation.sin()),
        Dimension::Aligned(d) => {
            let dx = d.second_point.x - d.first_point.x;
            let dy = d.second_point.y - d.first_point.y;
            let length = (dx * dx + dy * dy).sqrt();
            (d.definition_point, dx / length, dy / length)
        }
        other => panic!("not a linear dimension: {other:?}"),
    };
    if ax < -1e-12 || (ax.abs() <= 1e-12 && ay < 0.0) {
        ax = -ax;
        ay = -ay;
    }
    ax * (text.y - on_line.y) - ay * (text.x - on_line.x)
}

fn expected_lift(scale: f64) -> f64 {
    (GAP + TEXT / 2.0) * scale
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn dimlinear_text_is_lifted_by_the_style_at_every_scale() {
    for scale in [1.0, 8.0, 48.0] {
        let mut app = fresh_app();
        arch_style(&mut app, scale);
        run_ok(&mut app, "DIMLINEAR 0,0 100,0 H 50,20");
        let (_, dimension) = only_dimension(&app);
        let text = dimension.base().text_middle_point;
        assert!(
            close(lift(&dimension), expected_lift(scale)),
            "1:{scale}: text {text:?} is {} off the line, expected {}",
            lift(&dimension),
            expected_lift(scale)
        );
        assert!(close(text.x, 50.0), "1:{scale}: text centred, got {text:?}");
        assert!(close(text.y, 20.0 + expected_lift(scale)), "above the line");
        assert_eq!(dimension.base().insertion_point, text);
        assert!(!dimension.base().text_user_positioned);
    }
}

#[test]
fn vertical_and_aligned_text_sit_on_the_reading_side() {
    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMLINEAR 0,0 0,90 V -30,45");
    let (_, vertical) = only_dimension(&app);
    let text = vertical.base().text_middle_point;
    assert!(close(lift(&vertical), expected_lift(8.0)), "vertical: {text:?}");
    assert!(close(text.x, -30.0 - expected_lift(8.0)), "left of the line: {text:?}");
    assert!(close(text.y, 45.0));

    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMALIGNED 0,0 60,80 -40,30");
    let (_, aligned) = only_dimension(&app);
    assert!(
        close(lift(&aligned), expected_lift(8.0)),
        "aligned: text {:?} is {} off the line",
        aligned.base().text_middle_point,
        lift(&aligned)
    );
}

#[test]
fn text_that_does_not_fit_goes_past_the_extension_line_as_drawn() {
    // 1 1/2" at 1:8 (A-501, "1 1/2" RECOUVR."): the text cannot sit between
    // the extension lines. The stored point is the one the picture uses.
    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMLINEAR 0,0 1.5,0 H 0.75,10");
    let (handle, dimension) = only_dimension(&app);
    let text = dimension.base().text_middle_point;
    assert!(text.x > 1.5, "outside the second extension line: {text:?}");
    assert!(close(text.y, 10.0 + expected_lift(8.0)), "still above: {text:?}");
    let doc = &app.tabs[app.active_tab].scene.document;
    let EntityType::Dimension(stored) = doc.get_entity(handle).unwrap() else {
        panic!()
    };
    assert_eq!(
        crate::entities::dimension::automatic_text_point(stored, doc),
        Some(text),
        "creation and regeneration agree"
    );
}

#[test]
fn an_override_that_changes_the_text_re_places_it() {
    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMLINEAR 0,0 100,0 H 50,20");
    let (handle, _) = only_dimension(&app);
    let i = app.active_tab;

    // A larger gap lifts the text by the new DIMGAP × DIMSCALE.
    assert!(dim_override::set_property(
        &mut app.tabs[i].scene.document,
        handle,
        "dim_text_offset",
        "0.0625"
    ));
    let (_, dimension) = only_dimension(&app);
    assert!(
        close(lift(&dimension), (0.0625 + TEXT / 2.0) * 8.0),
        "new gap: {}",
        lift(&dimension)
    );

    // A colour or a line type leaves the text where it is, even a text the
    // style would now put elsewhere (as a drawing from another program).
    let Some(EntityType::Dimension(stored)) = app.tabs[i].scene.document.get_entity_mut(handle)
    else {
        panic!()
    };
    stored.base_mut().text_middle_point = Vector3::new(51.0, 21.0, 0.0);
    assert!(dim_override::set_property(
        &mut app.tabs[i].scene.document,
        handle,
        "dim_ext_line_offset",
        "0.125"
    ));
    let (_, dimension) = only_dimension(&app);
    assert_eq!(dimension.base().text_middle_point, Vector3::new(51.0, 21.0, 0.0));

    // The ERP's format (feet and inches) regenerates it.
    for (field, value) in [
        ("dim_units", "Architectural"),
        ("dim_precision", "0.0000"),
        ("dim_suppress_zero_feet", "Yes"),
        ("dim_suppress_zero_inches", "No"),
        ("dim_fractional_type", "Not stacked"),
    ] {
        assert!(
            dim_override::set_property(&mut app.tabs[i].scene.document, handle, field, value),
            "{field}"
        );
    }
    let (_, dimension) = only_dimension(&app);
    assert!(close(lift(&dimension), (0.0625 + TEXT / 2.0) * 8.0));
    assert!(close(dimension.base().text_middle_point.x, 50.0));
}

#[test]
fn text_placed_by_hand_stays_put() {
    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMLINEAR 0,0 100,0 H 50,20");
    let (handle, _) = only_dimension(&app);
    let i = app.active_tab;
    let Some(EntityType::Dimension(stored)) = app.tabs[i].scene.document.get_entity_mut(handle)
    else {
        panic!()
    };
    stored.base_mut().text_user_positioned = true;
    stored.base_mut().text_middle_point = Vector3::new(70.0, 30.0, 0.0);
    assert!(dim_override::set_property(
        &mut app.tabs[i].scene.document,
        handle,
        "dim_text_height",
        "0.25"
    ));
    run_ok(&mut app, "DIMSTYLE SET Standard dimscale 48");
    let (_, dimension) = only_dimension(&app);
    assert_eq!(dimension.base().text_middle_point, Vector3::new(70.0, 30.0, 0.0));
}

#[test]
fn a_style_edit_regenerates_the_text_and_undo_restores_it() {
    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMLINEAR 0,0 100,0 H 50,20");
    let (_, before) = only_dimension(&app);
    assert!(close(lift(&before), expected_lift(8.0)));

    run_ok(&mut app, "DIMSTYLE SET Standard dimscale 24");
    let (_, after) = only_dimension(&app);
    assert!(
        close(lift(&after), expected_lift(24.0)),
        "1:24 after the edit: {}",
        lift(&after)
    );

    assert_eq!(app.automation_op(r#"{"op":"undo"}"#)["ok"], true);
    let (_, undone) = only_dimension(&app);
    assert!(
        close(lift(&undone), expected_lift(8.0)),
        "undo: {}",
        lift(&undone)
    );
}

#[test]
fn a_saved_drawing_keeps_the_text_where_it_was_drawn() {
    let dir = std::env::temp_dir().join(format!("ocs-dimtext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cotes.dwg");
    let path = path.to_string_lossy().replace('\\', "/");

    let mut app = fresh_app();
    arch_style(&mut app, 8.0);
    run_ok(&mut app, "DIMLINEAR 0,0 100,0 H 50,20");
    let saved = app.automation_op(&json!({ "op": "save", "path": path }).to_string());
    assert_eq!(saved["ok"], true, "{saved}");

    let mut reopened = fresh_app();
    let opened = reopened.automation_op(&json!({ "op": "open", "path": path }).to_string());
    assert_eq!(opened["ok"], true, "{opened}");
    let (_, dimension) = only_dimension(&reopened);
    assert!(
        close(lift(&dimension), expected_lift(8.0)),
        "group 11 after a round trip: {:?}",
        dimension.base().text_middle_point
    );
    assert!(
        !dimension.base().block_name.is_empty(),
        "the saved picture comes back with the dimension"
    );

    // Editing the style of a saved drawing, as the ERP does on every call,
    // drops the picture made under the old settings and lifts the text anew.
    run_ok(&mut reopened, "DIMSTYLE SET Standard dimscale 24");
    let (_, edited) = only_dimension(&reopened);
    assert!(edited.base().block_name.is_empty(), "stale picture dropped");
    assert!(
        close(lift(&edited), expected_lift(24.0)),
        "1:24 after reopening: {}",
        lift(&edited)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
