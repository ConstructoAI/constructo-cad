//! Linetypes and hatch patterns of a drawing in inches: the imperial catalog
//! (acad.lin, acad.pat), never the metric one 25.4 times too coarse, and
//! never the catalog AND a compensating LTSCALE.

use super::OpenCADStudio;
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

fn hidden(app: &OpenCADStudio) -> Vec<f64> {
    app.tabs[app.active_tab]
        .scene
        .document
        .line_types
        .get("HIDDEN")
        .expect("HIDDEN is a standard linetype")
        .elements
        .iter()
        .map(|element| element.length)
        .collect()
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

fn ltscale(app: &OpenCADStudio) -> f64 {
    app.tabs[app.active_tab].scene.document.header.linetype_scale
}

fn temp_dwg(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("ocs-units-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.dwg")).to_string_lossy().replace('\\', "/");
    (dir, path)
}

/// The ERP opens a new drawing and makes it a drawing in inches: HIDDEN
/// becomes the 1/4" dash of acad.lin, LTSCALE stays what it was, and the
/// drawing saves and reopens that way.
#[test]
fn a_new_drawing_in_inches_gets_the_imperial_linetypes() {
    let mut app = fresh_app();
    assert!(close(&hidden(&app), &[6.35, -3.175]), "metric until told otherwise");
    run_ok(&mut app, "INSUNITS 1");
    assert!(close(&hidden(&app), &[0.25, -0.125]), "{:?}", hidden(&app));
    assert_eq!(ltscale(&app), 1.0, "the catalog is the one fix");
    run_ok(&mut app, "INSUNITS 1");
    assert!(close(&hidden(&app), &[0.25, -0.125]), "once only");

    let (dir, path) = temp_dwg("pouces");
    assert_eq!(
        app.automation_op(&json!({ "op": "save", "path": path }).to_string())["ok"],
        true
    );
    let mut reopened = fresh_app();
    assert_eq!(
        reopened.automation_op(&json!({ "op": "open", "path": path }).to_string())["ok"],
        true
    );
    assert!(close(&hidden(&reopened), &[0.25, -0.125]), "{:?}", hidden(&reopened));
    let _ = std::fs::remove_dir_all(&dir);
}

/// CAD-000019 was made before the fix: a drawing in inches whose standard
/// linetypes are metric, compensated by LTSCALE 0.472441. Reopened and sent
/// INSUNITS 1 again, as the ERP does on every call, it keeps both.
#[test]
fn a_drawing_that_compensated_with_ltscale_is_left_alone() {
    let mut app = fresh_app();
    let i = app.active_tab;
    app.tabs[i].scene.document.header.insertion_units = 1;
    run_ok(&mut app, "LTSCALE 0.472441");
    assert!(close(&hidden(&app), &[6.35, -3.175]));

    let (dir, path) = temp_dwg("cad19");
    assert_eq!(
        app.automation_op(&json!({ "op": "save", "path": path }).to_string())["ok"],
        true
    );
    let mut reopened = fresh_app();
    assert_eq!(
        reopened.automation_op(&json!({ "op": "open", "path": path }).to_string())["ok"],
        true
    );
    run_ok(&mut reopened, "INSUNITS 1");
    assert!(close(&hidden(&reopened), &[6.35, -3.175]), "{:?}", hidden(&reopened));
    assert!((ltscale(&reopened) - 0.472441).abs() < 1e-9);
    let _ = std::fs::remove_dir_all(&dir);
}

fn first_hatch_spacing(app: &OpenCADStudio) -> f64 {
    let hatch = app.tabs[app.active_tab]
        .scene
        .document
        .entities()
        .find_map(|entity| match entity {
            EntityType::Hatch(hatch) => Some(hatch.clone()),
            _ => None,
        })
        .expect("a hatch");
    let line = &hatch.pattern.lines[0];
    line.offset.x.hypot(line.offset.y)
}

/// ANSI31 in a drawing in inches: lines 1/8" apart (× the scale asked), as
/// the reference sheets draw it, where it used to be 3.175".
#[test]
fn a_hatch_in_inches_uses_acad_pat() {
    let mut app = fresh_app();
    run_ok(&mut app, "INSUNITS 1");
    run_ok(&mut app, "HATCH PANSI31 S 0,0 10,0 10,10 0,10");
    let spacing = first_hatch_spacing(&app);
    assert!((spacing - 0.125).abs() < 1e-6, "ANSI31 spacing {spacing}");

    let mut metric = fresh_app();
    run_ok(&mut metric, "HATCH PANSI31 S 0,0 100,0 100,100 0,100");
    let spacing = first_hatch_spacing(&metric);
    assert!((spacing - 3.175).abs() < 1e-5, "metric drawing unchanged: {spacing}");
}

/// C-16: TrueType styles without a screen, as the reference sheets use them
/// (ARCH = arial.ttf, ARCH-GRAS = arialbd.ttf). Each style has a handle of its
/// own, STYLE SET makes one current for TEXT, and the DWG keeps it all.
#[test]
fn truetype_styles_are_made_and_saved_without_a_screen() {
    let mut app = fresh_app();
    run_ok(&mut app, "STYLE NEW ARCH arial.ttf");
    run_ok(&mut app, "STYLE NEW ARCH-GRAS");
    run_ok(&mut app, "STYLE FONT ARCH-GRAS arialbd.ttf");
    run_ok(&mut app, "STYLE SET ARCH-GRAS");
    {
        let doc = &app.tabs[app.active_tab].scene.document;
        let arch = doc.text_styles.get("ARCH").expect("ARCH");
        let bold = doc.text_styles.get("ARCH-GRAS").expect("ARCH-GRAS");
        assert_eq!(arch.font_file, "arial.ttf");
        assert_eq!(bold.font_file, "arialbd.ttf");
        assert!(!arch.handle.is_null() && !bold.handle.is_null());
        assert_ne!(arch.handle, bold.handle);
        assert_eq!(doc.header.current_text_style_name, "ARCH-GRAS");
        assert_eq!(doc.header.current_text_style_handle, bold.handle);
    }
    run_ok(&mut app, "TEXT 0,0 1.25 0 MARCHE SUR LIMON");
    let style_of_text = |app: &OpenCADStudio| {
        app.tabs[app.active_tab]
            .scene
            .document
            .entities()
            .find_map(|entity| match entity {
                EntityType::Text(text) => Some(text.style.clone()),
                _ => None,
            })
    };
    assert_eq!(style_of_text(&app).as_deref(), Some("ARCH-GRAS"));

    let (dir, path) = temp_dwg("styles");
    assert_eq!(
        app.automation_op(&json!({ "op": "save", "path": path }).to_string())["ok"],
        true
    );
    let mut reopened = fresh_app();
    assert_eq!(
        reopened.automation_op(&json!({ "op": "open", "path": path }).to_string())["ok"],
        true
    );
    let doc = &reopened.tabs[reopened.active_tab].scene.document;
    assert_eq!(doc.text_styles.get("ARCH").map(|s| s.font_file.as_str()), Some("arial.ttf"));
    assert_eq!(
        doc.text_styles.get("ARCH-GRAS").map(|s| s.font_file.as_str()),
        Some("arialbd.ttf")
    );
    assert_eq!(style_of_text(&reopened).as_deref(), Some("ARCH-GRAS"));
    let _ = std::fs::remove_dir_all(&dir);
}
