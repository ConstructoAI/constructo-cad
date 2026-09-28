//! One-line commands driven without a screen — what the ERP's CAD module sends
//! the engine (`{"op":"run","cmd":"..."}`), one complete command per line.
//!
//! Each case here was measured failing against the production engine
//! (00daed78) before its fix: it drew nothing, dropped geometry, or left the
//! command open so that every following request was refused with
//! `command_busy`.

use super::OpenCADStudio;
use acadrust::EntityType;
use serde_json::Value;

fn app() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], true);
    app
}

/// Runs one command line headlessly and returns the engine's response.
fn run(app: &mut OpenCADStudio, cmd: &str) -> Value {
    let request = serde_json::json!({ "op": "run", "cmd": cmd }).to_string();
    app.automation_op(&request)
}

/// Runs one line that must succeed and leave no command open.
fn run_done(app: &mut OpenCADStudio, cmd: &str) -> Value {
    let response = run(app, cmd);
    assert_eq!(response["ok"], true, "{cmd}: {response}");
    assert_eq!(response["status"], "completed", "{cmd} left a command open: {response}");
    let i = app.active_tab;
    assert!(app.tabs[i].active_cmd.is_none(), "{cmd} left a command active");
    response
}

fn entities(app: &OpenCADStudio) -> Vec<EntityType> {
    app.tabs[app.active_tab]
        .scene
        .document
        .entities()
        .cloned()
        .collect()
}

fn polylines(app: &OpenCADStudio) -> Vec<acadrust::LwPolyline> {
    entities(app)
        .into_iter()
        .filter_map(|entity| match entity {
            EntityType::LwPolyline(polyline) => Some(polyline),
            _ => None,
        })
        .collect()
}

fn lines(app: &OpenCADStudio) -> Vec<acadrust::Line> {
    entities(app)
        .into_iter()
        .filter_map(|entity| match entity {
            EntityType::Line(line) => Some(line),
            _ => None,
        })
        .collect()
}

fn circles(app: &OpenCADStudio) -> Vec<acadrust::Circle> {
    entities(app)
        .into_iter()
        .filter_map(|entity| match entity {
            EntityType::Circle(circle) => Some(circle),
            _ => None,
        })
        .collect()
}

fn mtexts(app: &OpenCADStudio) -> Vec<acadrust::MText> {
    entities(app)
        .into_iter()
        .filter_map(|entity| match entity {
            EntityType::MText(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn texts(app: &OpenCADStudio) -> Vec<acadrust::Text> {
    entities(app)
        .into_iter()
        .filter_map(|entity| match entity {
            EntityType::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn count(app: &OpenCADStudio, keep: fn(&EntityType) -> bool) -> usize {
    entities(app).iter().filter(|entity| keep(entity)).count()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// ── PLINE in a vertical UCS (constat C-01) ──────────────────────────────────

#[test]
fn pline_in_a_vertical_ucs_keeps_a_rectangle_whole() {
    // UCS 3P through X and Z: the drawing plane stands on edge, and every
    // point of the rectangle's left side has the world X and Y of its first
    // vertex. The close test compared world X/Y and shut the polyline at the
    // third point — a rectangle became a triangle.
    let mut app = app();
    run_done(&mut app, "UCS 3POINT 0,0,0 1,0,0 0,0,1");
    run_done(&mut app, "PLINE 0,0 10,0 10,5 0,5 C");
    let polylines = polylines(&app);
    assert_eq!(polylines.len(), 1);
    assert_eq!(polylines[0].vertices.len(), 4, "every corner kept");
    assert!(polylines[0].is_closed, "C closes it");
}

#[test]
fn pline_in_a_vertical_ucs_keeps_a_stair_profile_whole() {
    // A stringer profile whose fourth vertex sits right above the first one:
    // same world X and Y, 14 units higher.
    let mut app = app();
    run_done(&mut app, "UCS 3POINT 0,0,0 1,0,0 0,0,1");
    run_done(&mut app, "PLINE 10,0 20,0 20,14 10,14 10,7 0,7 0,0 C");
    let polylines = polylines(&app);
    assert_eq!(polylines.len(), 1);
    assert_eq!(polylines[0].vertices.len(), 7, "every step kept");
    assert!(polylines[0].is_closed);
}

#[test]
fn pline_still_closes_on_its_first_vertex_in_a_vertical_ucs() {
    // The #421 behaviour stays: a point that really lands back on the first
    // vertex — in the drawing plane — closes the polyline without a
    // duplicate vertex.
    let mut app = app();
    run_done(&mut app, "UCS 3POINT 0,0,0 1,0,0 0,0,1");
    run_done(&mut app, "PLINE 0,0 10,0 10,5 0,5 0,0");
    let polylines = polylines(&app);
    assert_eq!(polylines.len(), 1);
    assert_eq!(polylines[0].vertices.len(), 4);
    assert!(polylines[0].is_closed);
}

// ── MTEXT: the rest of the line is the text ─────────────────────────────────

#[test]
fn mtext_on_one_line_creates_its_text() {
    // Production: `added: 0`, the editor left open, the text lost.
    let mut app = app();
    let response = run_done(&mut app, "MTEXT 0,0 40,-10 Bonjour le monde");
    assert_eq!(response["added"], 1, "{response}");
    let texts = mtexts(&app);
    assert_eq!(texts.len(), 1);
    assert!(texts[0].value.contains("Bonjour le monde"), "{:?}", texts[0].value);
    assert!(app.mtext_editor.is_none(), "the editor closes with the line");
}

#[test]
fn mtext_options_before_the_corner_and_paragraph_breaks() {
    let mut app = app();
    run_done(&mut app, r"MTEXT 0,-20 J MC H 2.5 40,-30 Deux\Plignes");
    let texts = mtexts(&app);
    assert_eq!(texts.len(), 1);
    assert!(close(texts[0].height, 2.5), "H option: {}", texts[0].height);
    assert!(
        matches!(
            texts[0].attachment_point,
            acadrust::entities::mtext::AttachmentPoint::MiddleCenter
        ),
        "J option"
    );
    let value = &texts[0].value;
    assert!(
        value.contains("Deux") && value.contains("lignes") && value.contains("\\P"),
        "{value:?}"
    );
}

#[test]
fn mtext_without_text_keeps_its_editor_as_before() {
    // The line gave no text: the editor stays open for someone to type in —
    // the GUI behaviour, unchanged. Nothing is created.
    let mut app = app();
    let response = run(&mut app, "MTEXT 0,0 40,-10");
    assert_eq!(response["added"], 0);
    assert!(app.mtext_editor.is_some());
}

// ── TEXT on one line (the same mechanism) ───────────────────────────────────

#[test]
fn text_on_one_line_creates_its_text_and_keeps_the_spacing() {
    // Production: `added: 0`, the in-place editor left open.
    let mut app = app();
    let response = run_done(&mut app, "TEXT J MC 50,0 2.5 0 Texte  centre au milieu");
    assert_eq!(response["added"], 1, "{response}");
    let texts = texts(&app);
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].value, "Texte  centre au milieu", "verbatim, double space kept");
    assert!(matches!(
        texts[0].horizontal_alignment,
        acadrust::entities::TextHorizontalAlignment::Center
    ));
    assert!(matches!(
        texts[0].vertical_alignment,
        acadrust::entities::TextVerticalAlignment::Middle
    ));
    assert!(close(texts[0].height, 2.5));
    assert!(app.text_inline.is_none(), "the next-line editor is closed too");
}

// ── LEADER, DIMCONTINUE, DIMBASELINE, QDIM: the line ends the command ───────

#[test]
fn leader_on_one_line_places_its_note_and_ends() {
    // Production: `waiting_input`, then `command_busy` for what followed.
    let mut app = app();
    let response = run_done(&mut app, "LEADER 0,0 10,10 Note de chantier");
    assert_eq!(response["added"], 2, "leader and its note: {response}");
    assert_eq!(mtexts(&app)[0].value, "Note de chantier");
}

#[test]
fn leader_annotation_keyword_takes_the_whole_rest_of_the_line() {
    // Production split the note into one line per word ("Voir\ndetail\n3").
    let mut app = app();
    run_done(&mut app, "LEADER 50,0 60,10 A Voir detail 3");
    assert_eq!(mtexts(&app)[0].value, "Voir detail 3");
}

#[test]
fn leader_without_a_note_is_placed_alone() {
    let mut app = app();
    let response = run_done(&mut app, "LEADER 100,0 110,10");
    assert_eq!(response["added"], 1);
    assert_eq!(count(&app, |e| matches!(e, EntityType::Leader(_))), 1);
    assert!(mtexts(&app).is_empty());
}

#[test]
fn dimcontinue_and_dimbaseline_end_with_their_line() {
    // Production: both left open (`waiting_input`), and the control channel
    // refused the next request with `command_busy`.
    let mut app = app();
    run_done(&mut app, "LINE 0,0 100,0");
    run_done(&mut app, "DIMLINEAR 0,0 20,0 10,10");
    let response = run_done(&mut app, "DIMCONTINUE 40,0 60,0");
    assert_eq!(response["added"], 2);
    run_done(&mut app, "DIMLINEAR 0,0 20,0 10,-10");
    let response = run_done(&mut app, "DIMBASELINE 40,0 60,0");
    assert_eq!(response["added"], 2);
    assert_eq!(count(&app, |e| matches!(e, EntityType::Dimension(_))), 6);
}

#[test]
fn qdim_selects_then_places_on_one_line() {
    // Production: the position point was swallowed by "Select geometry",
    // nothing placed, the command left open.
    let mut app = app();
    for x in [0, 20, 50] {
        run_done(&mut app, &format!("LINE {x},0 {x},10"));
    }
    let response = run_done(&mut app, "QDIM ALL 25,-10");
    assert_eq!(response["added"], 2, "{response}");
    let response = run_done(&mut app, "QDIM ALL");
    assert_eq!(response["added"], 0, "no position: nothing placed, command closed");
}

// ── Modifying on L: a value ends the selection ──────────────────────────────

#[test]
fn move_last_on_one_line() {
    // Production: MOVE left waiting for its base point, the line not moved.
    let mut app = app();
    run_done(&mut app, "LINE 0,0 10,0");
    run_done(&mut app, "MOVE L 0,0 0,5");
    let lines = lines(&app);
    assert_eq!(lines.len(), 1);
    assert!(close(lines[0].start.y, 5.0) && close(lines[0].end.y, 5.0), "{:?}", lines[0]);
}

#[test]
fn copy_last_on_one_line() {
    let mut app = app();
    run_done(&mut app, "LINE 0,0 10,0");
    run_done(&mut app, "COPY L 0,0 0,-5");
    let lines = lines(&app);
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|line| close(line.start.y, -5.0)), "{lines:?}");
}

#[test]
fn rotate_and_scale_last_on_one_line() {
    let mut app = app();
    run_done(&mut app, "LINE 0,0 10,0");
    run_done(&mut app, "ROTATE L 0,0 90");
    let line = lines(&app).remove(0);
    assert!(close(line.end.x, 0.0) && close(line.end.y, 10.0), "{line:?}");

    let mut app = self::app();
    run_done(&mut app, "LINE 0,0 10,0");
    run_done(&mut app, "SCALE L 0,0 2");
    let line = lines(&app).remove(0);
    assert!(close(line.end.x, 20.0), "{line:?}");
}

#[test]
fn offset_takes_last_or_a_typed_point_at_its_object_prompt() {
    // Production: `OFFSET 2 L 60,0` answered ok and made nothing.
    let mut app = app();
    run_done(&mut app, "CIRCLE 50,0 5");
    run_done(&mut app, "OFFSET 2 L 60,0");
    let radii: Vec<f64> = circles(&app).iter().map(|c| c.radius).collect();
    assert!(radii.iter().any(|r| close(*r, 7.0)), "{radii:?}");

    let mut app = self::app();
    run_done(&mut app, "CIRCLE 50,0 5");
    run_done(&mut app, "OFFSET 2 55,0 60,0");
    let radii: Vec<f64> = circles(&app).iter().map(|c| c.radius).collect();
    assert!(radii.iter().any(|r| close(*r, 7.0)), "{radii:?}");
}

#[test]
fn trim_takes_a_typed_point_on_the_piece_to_remove() {
    // Production: `TRIM L 115,0` answered ok and trimmed nothing; a script
    // had no way to say which piece goes.
    let mut app = app();
    run_done(&mut app, "LINE 100,0 120,0");
    run_done(&mut app, "LINE 110,-5 110,5");
    run_done(&mut app, "TRIM 115,0");
    let lines = lines(&app);
    assert!(
        !lines
            .iter()
            .any(|line| close(line.start.x, 120.0) || close(line.end.x, 120.0)),
        "the piece right of the cut is gone: {lines:?}"
    );
    assert!(lines.iter().any(|line| {
        close(line.start.x.min(line.end.x), 100.0) && close(line.start.x.max(line.end.x), 110.0)
    }));
}

// ── Current colour / lineweight, set the way the ERP sets them ──────────────

static REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A control-channel request (`protocol: 1`), as the ERP's `cad_outils` sends
/// for what the command line cannot say (true colour, lineweight).
fn control(app: &mut OpenCADStudio, mut request: Value) -> Value {
    let id = REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    request["protocol"] = serde_json::json!(1);
    request["request_id"] = serde_json::json!(format!("headless-{id}"));
    request["document_id"] = serde_json::json!(app.tabs[app.active_tab].id);
    let response = app.automation_op(&request.to_string());
    assert_eq!(response["ok"], true, "{request}: {response}");
    response
}

fn set_current_colour_and_lineweight(app: &mut OpenCADStudio, rgb: [u8; 3], lineweight: i16) {
    control(
        app,
        serde_json::json!({"op": "property", "field": "color", "value": {"rgb": rgb}}),
    );
    control(
        app,
        serde_json::json!({"op": "property", "field": "line_weight", "value": lineweight}),
    );
}

#[test]
fn a_new_hatch_takes_the_current_colour_and_lineweight() {
    // Production: a hatch was left ByLayer whatever colour and lineweight
    // were current — a grey poché came out in the layer's colour.
    let mut app = app();
    set_current_colour_and_lineweight(&mut app, [128, 128, 128], 13);
    run_done(&mut app, "HATCH PSOLID S 0,0 10,0 10,10 0,10");
    let hatch = entities(&app)
        .into_iter()
        .find(|entity| matches!(entity, EntityType::Hatch(_)))
        .expect("a hatch");
    assert_eq!(hatch.common().color, acadrust::types::Color::from_rgb(128, 128, 128));
    assert_eq!(hatch.common().line_weight, acadrust::types::LineWeight::Value(13));
}

#[test]
fn true_colours_and_lineweights_survive_a_dwg_round_trip() {
    // What a plan needs to come back identical from the ERP: the entity's and
    // the layer's true colour and lineweight, written in the DWG and read back.
    let mut app = app();
    run_done(&mut app, "LAYER NEW MURS");
    control(
        &mut app,
        serde_json::json!({"op": "set_properties", "collection": "layers", "name": "MURS",
            "updates": [{"path": "/line_weight", "value": {"Value": 70}},
                        {"path": "/color", "value": {"Rgb": {"r": 10, "g": 120, "b": 200}}}]}),
    );
    set_current_colour_and_lineweight(&mut app, [200, 30, 40], 50);
    run_done(&mut app, "LINE 0,0 100,0");
    set_current_colour_and_lineweight(&mut app, [128, 128, 128], 13);
    run_done(&mut app, "HATCH PSOLID S 0,20 100,20 100,40 0,40");

    let path = std::env::temp_dir().join(format!(
        "ocs_headless_colours_{}.dwg",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().replace('\\', "\\\\");
    assert_eq!(app.automation_op(&format!(r#"{{"op":"save","path":"{p}"}}"#))["ok"], true);
    let mut reread = OpenCADStudio::new_for_test();
    let opened = reread.automation_op(&format!(r#"{{"op":"open","path":"{p}"}}"#));
    assert_eq!(opened["ok"], true, "{opened}");

    let document = &reread.tabs[reread.active_tab].scene.document;
    let layer = document.layers.get("MURS").expect("layer MURS");
    assert_eq!(layer.color, acadrust::types::Color::from_rgb(10, 120, 200));
    assert_eq!(layer.line_weight, acadrust::types::LineWeight::Value(70));
    let line = document
        .entities()
        .find(|entity| matches!(entity, EntityType::Line(_)))
        .expect("the line");
    assert_eq!(line.common().color, acadrust::types::Color::from_rgb(200, 30, 40));
    assert_eq!(line.common().line_weight, acadrust::types::LineWeight::Value(50));
    let hatch = document
        .entities()
        .find(|entity| matches!(entity, EntityType::Hatch(_)))
        .expect("the hatch");
    assert_eq!(hatch.common().color, acadrust::types::Color::from_rgb(128, 128, 128));
    assert_eq!(hatch.common().line_weight, acadrust::types::LineWeight::Value(13));

    drop(app);
    drop(reread);
    let sidecar = path.with_file_name(format!(
        ".{}.ocs.lock",
        path.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_file(sidecar);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn last_selects_a_hatch_too() {
    // Production: L skipped fills — it took the last object that draws a
    // wire, so `ERASE L` after a hatch erased the rectangle under it.
    let mut app = app();
    run_done(&mut app, "RECTANG 0,0 10,10");
    run_done(&mut app, "HATCH PANSI31 5,5");
    assert_eq!(count(&app, |e| matches!(e, EntityType::Hatch(_))), 1);
    run_done(&mut app, "ERASE L");
    assert_eq!(count(&app, |e| matches!(e, EntityType::Hatch(_))), 0, "the hatch went");
    assert_eq!(polylines(&app).len(), 1, "the rectangle stayed");
}
