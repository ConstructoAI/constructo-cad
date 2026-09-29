//! Paper layouts and floating viewports from one command line each.
//!
//! The headless automation server (`--serve`) runs one command line per
//! request and presses Enter once. Until now the only ways to create or name a
//! layout, give it a sheet, or aim a viewport at a model area at an exact scale
//! were the layout tabs, the Layout Manager, the page-setup dialog and a mouse
//! zoom inside the viewport — none of which exist without a window. `LAYOUTTAB`
//! only toggles the tab strip, and `MVIEW` refuses to run in Model space, so a
//! drawing set built by a script had to lay every sheet out side by side in
//! model space. Each of those actions gets a one-line form here:
//!
//! ```text
//! LAYOUT N <name> [<width> <height> [IN|MM] | <paper> [LANDSCAPE|PORTRAIT]]
//! LAYOUT S <name>                    (Model included)
//! LAYOUT R <old> <new>
//! LAYOUT D <name>
//! LAYOUT P <name> <width> <height> [IN|MM]   (or a named paper)
//! LAYOUT ?
//! MVIEW <x0,y0> <x1,y1> [CENTER <x,y>] [HEIGHT <h> | SCALE <s>] [LOCK] [LAYER <name>]
//! LAYER PLOT <name> ON|OFF
//! ```
//!
//! `LAYOUT` follows the option letters of the command-line layout command of
//! the reference application (New, Set, Rename, Delete, `?`); `P` (Paper) is
//! ours, since that application only sizes a sheet through a dialog. A name
//! may be double-quoted to hold spaces. Everything a line asks for is checked
//! before the drawing changes, so a refused line changes nothing.
//!
//! The `MVIEW` corners are paper coordinates of the current layout; `CENTER`
//! is the model-space point (WCS, plan view) the viewport looks at; `HEIGHT`
//! is the model height the viewport shows, `SCALE` the paper length per model
//! length (`1/48`, `1:48`, `1/48XP`, `1/4"=1'-0"`). Without either, the view
//! fits the model extents exactly as the interactive `MVIEW` does.

use crate::app::{Message, OpenCADStudio};
use crate::io::paper_catalog::{self, PaperSize, PaperUnits};
use acadrust::objects::{Layout, ObjectType};
use acadrust::{CadDocument, EntityType};
use iced::Task;
use serde_json::{json, Value};

/// The one-line forms, as printed when `LAYOUT` gets no or a wrong option.
const LAYOUT_USAGE: &str = "LAYOUT N <name> [<width> <height> [IN|MM]] | S <name> | \
R <old> <new> | D <name> | P <name> <width> <height> [IN|MM] | ?";

const MVIEW_OPTIONS: &str = "CENTER <x,y>, HEIGHT <h>, SCALE <s>, LOCK or LAYER <name>";

/// A sheet asked for on the command line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SheetSpec {
    /// The medium, under its catalogue name (portrait dimensions).
    pub paper: PaperSize,
    /// Long side horizontal on the layout.
    pub landscape: bool,
    /// Unit of the layout's paper space: its limits and viewport corners.
    pub units: PaperUnits,
}

impl SheetSpec {
    /// Width and height of the sheet on the layout, in its paper units.
    pub fn size(&self) -> (f64, f64) {
        // Stay in the medium's own unit when it is the layout's: 24 × 36 in
        // must give 24 and 36, not 23.999… after a trip through millimetres.
        let (short, long) = if self.units == self.paper.units {
            (self.paper.width, self.paper.height)
        } else {
            let (short_mm, long_mm) = self.paper.portrait_mm();
            (self.units.from_mm(short_mm), self.units.from_mm(long_mm))
        };
        if self.landscape {
            (long, short)
        } else {
            (short, long)
        }
    }
}

/// A one-line `MVIEW`, parsed but not yet applied.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MviewSpec {
    pub first: (f64, f64),
    pub second: (f64, f64),
    /// Model-space point the view is centred on.
    pub center: Option<(f64, f64)>,
    /// Model height shown by the viewport.
    pub height: Option<f64>,
    /// Paper length per model length.
    pub scale: Option<f64>,
    pub locked: bool,
    pub layer: Option<String>,
}

impl MviewSpec {
    /// `(x0, y0, x1, y1)` with the corners in increasing order.
    fn rectangle(&self) -> (f64, f64, f64, f64) {
        (
            self.first.0.min(self.second.0),
            self.first.1.min(self.second.1),
            self.first.0.max(self.second.0),
            self.first.1.max(self.second.1),
        )
    }

    /// The view height the viewport gets, or `None` to keep the fit to the
    /// model extents.
    fn view_height(&self) -> Option<f64> {
        let (_, y0, _, y1) = self.rectangle();
        match (self.height, self.scale) {
            (Some(height), _) => Some(height),
            (None, Some(scale)) => Some((y1 - y0) / scale),
            (None, None) => None,
        }
    }
}

/// Split a command line on whitespace, keeping a double-quoted run as one
/// token (`LAYOUT N "Plan RDC"`). The quotes are dropped. A quote only opens
/// a token when it is the token's first character, so a length such as
/// `1'-0"` stays as typed.
pub(crate) fn split_args(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            break;
        };
        let mut token = String::new();
        if first == '"' {
            chars.next();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '"' {
                    closed = true;
                    break;
                }
                token.push(c);
            }
            if !closed {
                return Err("a quoted name is not closed.".to_string());
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err("a quoted name must be followed by a space.".to_string());
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
        }
        tokens.push(token);
    }
    Ok(tokens)
}

/// Case-insensitive name comparison, beyond ASCII (`É` = `é`).
fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn parse_paper_units(token: &str) -> Option<PaperUnits> {
    match token.trim().to_uppercase().as_str() {
        "IN" | "INCH" | "INCHES" | "\"" | "PO" | "POUCE" | "POUCES" => Some(PaperUnits::Inches),
        "MM" | "MILLIMETER" | "MILLIMETERS" | "MILLIMETRE" | "MILLIMETRES" => {
            Some(PaperUnits::Millimeters)
        }
        _ => None,
    }
}

/// `Some(true)` for landscape, `Some(false)` for portrait.
fn parse_orientation(token: &str) -> Option<bool> {
    match token.trim().to_uppercase().as_str() {
        "L" | "LANDSCAPE" | "PAYSAGE" => Some(true),
        "P" | "PORTRAIT" => Some(false),
        _ => None,
    }
}

/// The paper tokens that follow a layout name: `<width> <height> [IN|MM]` as
/// the sheet lies on the layout (landscape when wider than tall), or a paper
/// name the catalogue knows (`A3`, `Letter`, `ARCH_D_(24.00_x_36.00_Inches)`)
/// with an optional `LANDSCAPE` / `PORTRAIT` (landscape by default).
/// `default_units` applies to the first form when no unit is written.
pub(crate) fn parse_sheet(tokens: &[String], default_units: PaperUnits) -> Result<SheetSpec, String> {
    let number = |token: &str| crate::entities::common::parse_length(token);
    match tokens {
        [] => Err("a sheet size is required (<width> <height> [IN|MM]).".to_string()),
        [w, h, rest @ ..] if number(w.as_str()).is_some() && number(h.as_str()).is_some() => {
            let width = number(w.as_str()).unwrap_or(0.0);
            let height = number(h.as_str()).unwrap_or(0.0);
            if width <= 0.0 || height <= 0.0 {
                return Err(format!("the sheet size {w} x {h} must be positive."));
            }
            let units = match rest {
                [] => default_units,
                [unit] => parse_paper_units(unit)
                    .ok_or_else(|| format!("unknown paper unit \"{unit}\" (use IN or MM)."))?,
                _ => {
                    return Err(format!(
                        "unexpected \"{}\" after the sheet size.",
                        rest[1..].join(" ")
                    ))
                }
            };
            let paper = paper_catalog::match_dimensions_mm(units.to_mm(width), units.to_mm(height))
                .cloned()
                .unwrap_or_else(|| PaperSize::custom(width, height, units));
            Ok(SheetSpec {
                paper,
                landscape: width > height,
                units,
            })
        }
        [name, rest @ ..] => {
            let paper = paper_catalog::resolve(name)
                .ok_or_else(|| format!("unknown paper \"{name}\" (give <width> <height> [IN|MM])."))?;
            let landscape = match rest {
                [] => true,
                [orientation] => parse_orientation(orientation).ok_or_else(|| {
                    format!("unknown orientation \"{orientation}\" (use LANDSCAPE or PORTRAIT).")
                })?,
                _ => {
                    return Err(format!(
                        "unexpected \"{}\" after the paper name.",
                        rest[1..].join(" ")
                    ))
                }
            };
            Ok(SheetSpec {
                units: paper.units,
                paper,
                landscape,
            })
        }
    }
}

/// Paper length per model length: `1/48`, `1:48`, `1/48XP`, `0.25`, or a paper
/// length equal to a model length, `1/4"=1'-0"` or `1=100`.
pub(crate) fn parse_scale(token: &str) -> Option<f64> {
    use crate::entities::common::parse_length;
    let token = token.trim();
    let value = if let Some((paper, model)) = token.split_once('=') {
        let paper = parse_length(paper)?;
        let model = parse_length(model)?;
        if model.abs() < 1e-12 {
            return None;
        }
        paper / model
    } else if let Some((paper, model)) = token.split_once(':') {
        let paper: f64 = paper.trim().parse().ok()?;
        let model: f64 = model.trim().parse().ok()?;
        if model.abs() < 1e-12 {
            return None;
        }
        paper / model
    } else {
        let upper = token.to_ascii_uppercase();
        parse_length(upper.strip_suffix("XP").unwrap_or(upper.as_str()))?
    };
    (value.is_finite() && value > 0.0).then_some(value)
}

fn parse_point(token: &str) -> Option<(f64, f64)> {
    use crate::app::helpers::{parse_coord, CoordKind};
    let (point, kind) = parse_coord(token)?;
    (kind != CoordKind::Relative && point.x.is_finite() && point.y.is_finite())
        .then_some((point.x, point.y))
}

/// Whether a layer or layout name can be stored: not empty, no symbol-table
/// reserved character, at most 255 characters.
fn valid_symbol_name(name: &str) -> bool {
    name.chars().count() <= 255 && crate::scene::valid_block_name(name)
}

/// Parse the arguments of a one-line `MVIEW` (everything after the verb).
pub(crate) fn parse_mview(args: &[String]) -> Result<MviewSpec, String> {
    let [a, b, rest @ ..] = args else {
        return Err("MVIEW: two paper corners are required (x0,y0 x1,y1).".to_string());
    };
    let first =
        parse_point(a).ok_or_else(|| format!("MVIEW: \"{a}\" is not a paper corner (x,y)."))?;
    let second =
        parse_point(b).ok_or_else(|| format!("MVIEW: \"{b}\" is not a paper corner (x,y)."))?;
    let mut spec = MviewSpec {
        first,
        second,
        center: None,
        height: None,
        scale: None,
        locked: false,
        layer: None,
    };
    let mut k = 0;
    while k < rest.len() {
        let option = rest[k].to_uppercase();
        let value = rest.get(k + 1);
        let missing = || format!("MVIEW: {} needs a value.", rest[k]);
        match option.as_str() {
            "C" | "CENTER" | "CENTRE" => {
                let value = value.ok_or_else(missing)?;
                spec.center = Some(parse_point(value).ok_or_else(|| {
                    format!("MVIEW: \"{value}\" is not a model point (x,y).")
                })?);
                k += 2;
            }
            "H" | "HEIGHT" | "HAUTEUR" => {
                let value = value.ok_or_else(missing)?;
                let height = crate::entities::common::parse_length(value)
                    .filter(|height| *height > 0.0)
                    .ok_or_else(|| format!("MVIEW: \"{value}\" is not a positive view height."))?;
                spec.height = Some(height);
                k += 2;
            }
            "S" | "SCALE" | "ECHELLE" | "ÉCHELLE" => {
                let value = value.ok_or_else(missing)?;
                spec.scale = Some(parse_scale(value).ok_or_else(|| {
                    format!("MVIEW: \"{value}\" is not a scale (1/48, 1:48 or 1/4\"=1'-0\").")
                })?);
                k += 2;
            }
            "L" | "LOCK" | "VERROU" | "VERROUILLER" => {
                let switch = value.map(|value| value.to_uppercase());
                match switch.as_deref() {
                    Some("ON" | "YES" | "OUI") => {
                        spec.locked = true;
                        k += 2;
                    }
                    Some("OFF" | "NO" | "NON") => {
                        spec.locked = false;
                        k += 2;
                    }
                    _ => {
                        spec.locked = true;
                        k += 1;
                    }
                }
            }
            "LA" | "LAYER" | "CALQUE" => {
                let value = value.ok_or_else(missing)?;
                if !valid_symbol_name(value) {
                    return Err(format!("MVIEW: \"{value}\" is not a valid layer name."));
                }
                spec.layer = Some(value.clone());
                k += 2;
            }
            _ => {
                return Err(format!(
                    "MVIEW: unknown option \"{}\" (expected {MVIEW_OPTIONS}).",
                    rest[k]
                ))
            }
        }
    }
    if spec.height.is_some() && spec.scale.is_some() {
        return Err("MVIEW: give HEIGHT or SCALE, not both.".to_string());
    }
    let (x0, y0, x1, y1) = spec.rectangle();
    if x1 - x0 < 1e-6 || y1 - y0 < 1e-6 {
        return Err("MVIEW: the two corners do not enclose an area.".to_string());
    }
    Ok(spec)
}

/// The layout object called `name` (exact spelling). A drawing can carry a
/// placeholder of the same name without a block record; the real one wins,
/// as in [`crate::scene::Scene::layout_names`].
fn find_layout<'a>(document: &'a CadDocument, name: &str) -> Option<&'a Layout> {
    let mut found: Option<&Layout> = None;
    for object in document.objects.values() {
        if let ObjectType::Layout(layout) = object {
            if layout.name == name
                && found.is_none_or(|best| best.block_record.is_null() && !layout.block_record.is_null())
            {
                found = Some(layout);
            }
        }
    }
    found
}

fn find_layout_mut<'a>(document: &'a mut CadDocument, name: &str) -> Option<&'a mut Layout> {
    let handle = find_layout(document, name).map(|layout| layout.handle)?;
    match document.objects.get_mut(&handle) {
        Some(ObjectType::Layout(layout)) => Some(layout),
        _ => None,
    }
}

/// Give `layout` the sheet: medium name and size, orientation, paper units,
/// a 1:1 layout plot, no margins, and limits / extents covering the sheet.
/// The medium is stored the way the file format expects it (portrait
/// dimensions, landscape through the plot rotation), as the default page
/// setup does.
pub(crate) fn apply_sheet(layout: &mut Layout, sheet: &SheetSpec) {
    use acadrust::objects::{PlotPaperUnits, PlotRotation, PlotType, ScaledType};
    let (short_mm, long_mm) = sheet.paper.portrait_mm();
    let (width, height) = sheet.size();
    layout.paper_size = sheet.paper.canonical.to_string();
    layout.paper_width = short_mm;
    layout.paper_height = long_mm;
    layout.plot_rotation = if sheet.landscape {
        PlotRotation::Degrees90
    } else {
        PlotRotation::None
    }
    .to_code();
    layout.plot_paper_units = match sheet.units {
        PaperUnits::Inches => PlotPaperUnits::Inches,
        PaperUnits::Millimeters => PlotPaperUnits::Millimeters,
    }
    .to_code();
    layout.plot_type = PlotType::Layout.to_code();
    layout.plot_scale_type = ScaledType::OneToOne.to_code();
    layout.plot_scale_numerator = 1.0;
    layout.plot_scale_denominator = 1.0;
    layout.plot_scale_factor = 1.0;
    layout.plot_flags.use_standard_scale = true;
    layout.plot_margin_left = 0.0;
    layout.plot_margin_bottom = 0.0;
    layout.plot_margin_right = 0.0;
    layout.plot_margin_top = 0.0;
    layout.plot_origin_x = 0.0;
    layout.plot_origin_y = 0.0;
    layout.plot_window_min_x = 0.0;
    layout.plot_window_min_y = 0.0;
    layout.plot_window_max_x = 0.0;
    layout.plot_window_max_y = 0.0;
    layout.insertion_base = (0.0, 0.0, 0.0);
    layout.min_limits = (0.0, 0.0);
    layout.max_limits = (width, height);
    layout.min_extents = (0.0, 0.0, 0.0);
    layout.max_extents = (width, height, 0.0);
    // A layout read from DXF replays its raw plot-settings pairs on DXF
    // output; the fields above are now the truth.
    layout.raw_plot_settings_codes = None;
}

/// Make the layout's overall (sheet) viewport frame its limits, so the layout
/// opens on the whole sheet. The paper position goes in the view centre with
/// a zero target, as the paper-space camera stores it.
fn fit_sheet_viewport(scene: &mut crate::scene::Scene, layout_name: &str) {
    use acadrust::types::Vector3;
    scene.ensure_sheet_viewport(layout_name);
    let Some((sheet, min, max)) = find_layout(&scene.document, layout_name).map(|layout| {
        (
            crate::scene::Scene::layout_sheet_viewport_handle(&scene.document, layout),
            layout.min_limits,
            layout.max_limits,
        )
    }) else {
        return;
    };
    let (width, height) = ((max.0 - min.0).abs(), (max.1 - min.1).abs());
    if width < 1e-9 || height < 1e-9 {
        return;
    }
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    if let Some(EntityType::Viewport(viewport)) = scene.document.get_entity_mut(sheet) {
        viewport.center = Vector3::new(cx, cy, 0.0);
        viewport.width = width;
        viewport.height = height;
        viewport.view_target = Vector3::ZERO;
        viewport.view_center = Vector3::new(cx, cy, 0.0);
        viewport.view_direction = Vector3::UNIT_Z;
        viewport.twist_angle = 0.0;
        viewport.view_height = height * 1.1;
        viewport.custom_scale = 1.0;
    } else {
        return;
    }
    scene.bump_entities(&[(sheet, crate::scene::ChangeKind::Modified)]);
}

fn format_number(value: f64) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded}")
    }
}

fn paper_units_label(code: i16) -> &'static str {
    match code {
        0 => "in",
        1 => "mm",
        _ => "px",
    }
}

/// `ARCH D 36 x 24 in` — the medium's human name and the sheet as it lies on
/// the layout.
fn sheet_label(layout: &Layout) -> String {
    let width = (layout.max_limits.0 - layout.min_limits.0).abs();
    let height = (layout.max_limits.1 - layout.min_limits.1).abs();
    let name = paper_catalog::resolve(&layout.paper_size)
        .map(|paper| paper.label.into_owned())
        .unwrap_or_else(|| layout.paper_size.clone());
    format!(
        "{name} {} x {} {}",
        format_number(width),
        format_number(height),
        paper_units_label(layout.plot_paper_units)
    )
}

fn viewport_json(viewport: &acadrust::entities::Viewport) -> Value {
    let scale =
        crate::scene::vp_effective_scale(viewport.custom_scale, viewport.view_height, viewport.height);
    let scale_factor = (scale > 0.0).then(|| 1.0 / scale);
    let handle = format!("{:X}", viewport.common.handle.value());
    // Plan view without twist: the model point at the viewport's centre.
    let view_center = [
        viewport.view_target.x + viewport.view_center.x,
        viewport.view_target.y + viewport.view_center.y,
    ];
    json!({
        "handle": handle,
        "id": viewport.id,
        "layer": viewport.common.layer,
        "center": [viewport.center.x, viewport.center.y],
        "width": viewport.width,
        "height": viewport.height,
        "view_center": view_center,
        "view_height": viewport.view_height,
        "scale": scale,
        "scale_factor": scale_factor,
        "locked": viewport.status.locked,
        "on": viewport.status.is_on,
        "frozen_layers": viewport.frozen_layers.len(),
    })
}

impl OpenCADStudio {
    /// Whether `run_command_line` must hand this line to the dispatcher whole
    /// instead of starting the first word as an interactive tool and feeding
    /// it the rest: these lines parse their arguments together (quoted names,
    /// options after the corners), and the token feed would drop whatever a
    /// finished tool leaves unread.
    pub(crate) fn command_line_is_whole(&self, tokens: &[&str]) -> bool {
        let Some(first) = tokens.first() else {
            return false;
        };
        let verb = self
            .resolve_alias(first)
            .unwrap_or_else(|| first.to_string())
            .to_ascii_uppercase();
        match verb.as_str() {
            "LAYOUT" | "-LAYOUT" => true,
            "MVIEW" => {
                tokens.len() >= 3
                    && parse_point(tokens[1]).is_some()
                    && parse_point(tokens[2]).is_some()
            }
            "LAYER" | "-LAYER" => tokens.get(1).is_some_and(|word| word.eq_ignore_ascii_case("PLOT")),
            _ => false,
        }
    }

    /// Command family for `LAYOUT`, the one-line `MVIEW` and `LAYER PLOT`.
    /// Returns `None` for any other line so the next family can take it.
    pub(super) fn dispatch_layout_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let line = cmd.trim();
        let (verb, rest) = match line.split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb, rest.trim()),
            None => (line, ""),
        };
        match verb.to_ascii_uppercase().as_str() {
            "LAYOUT" | "-LAYOUT" => Some(self.layout_command(i, rest)),
            "MVIEW" if !rest.is_empty() => {
                // Only a line the interactive MVIEW did not take reaches here:
                // one routed whole by `run_command_line`, or any MVIEW line in
                // Model space, where the tool refuses to start.
                if self.tabs[i].scene.current_layout == "Model" {
                    self.command_line.push_error(
                        "MVIEW: switch to a paper space layout first (LAYOUT S <name> or CTAB <name>).",
                    );
                    return Some(Task::none());
                }
                let args = split_args(rest).ok()?;
                let corners = args.len() >= 2
                    && parse_point(&args[0]).is_some()
                    && parse_point(&args[1]).is_some();
                if !corners {
                    return None;
                }
                Some(self.mview_one_line(i, &args))
            }
            "LAYER" | "-LAYER" => {
                let args = split_args(rest).ok()?;
                if !args.first().is_some_and(|word| word.eq_ignore_ascii_case("PLOT")) {
                    return None;
                }
                Some(self.layer_plot_command(i, &args[1..]))
            }
            _ => None,
        }
    }

    fn layout_command(&mut self, i: usize, rest: &str) -> Task<Message> {
        let args = match split_args(rest) {
            Ok(args) => args,
            Err(error) => {
                self.command_line.push_error(&format!("LAYOUT: {error}"));
                return Task::none();
            }
        };
        let Some(option) = args.first() else {
            self.layout_list(i);
            self.command_line.push_info(LAYOUT_USAGE);
            return Task::none();
        };
        let tail = &args[1..];
        match option.to_uppercase().as_str() {
            "?" | "LIST" => {
                self.layout_list(i);
                Task::none()
            }
            "N" | "NEW" => {
                self.layout_new(i, tail);
                Task::none()
            }
            "S" | "SET" => self.layout_set(i, tail),
            "R" | "RENAME" => {
                self.layout_rename(i, tail);
                Task::none()
            }
            "D" | "DELETE" => self.layout_delete(i, tail),
            "P" | "PAPER" | "PAPIER" => {
                self.layout_paper(i, tail);
                Task::none()
            }
            _ => {
                self.command_line.push_error(&format!(
                    "LAYOUT: unknown option \"{option}\". Use {LAYOUT_USAGE}"
                ));
                Task::none()
            }
        }
    }

    /// Stored spelling of an existing layout, `Model` included.
    fn resolve_layout_name(&self, i: usize, name: &str) -> Option<String> {
        self.tabs[i]
            .scene
            .layout_names()
            .into_iter()
            .find(|candidate| same_name(candidate, name))
    }

    /// Why `name` cannot become a layout name, or `Ok`. `renaming` is the
    /// layout being renamed, which may keep its own name in another case.
    fn check_layout_name(&self, i: usize, name: &str, renaming: Option<&str>) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("a layout name is required.".to_string());
        }
        if !valid_symbol_name(name) {
            return Err(format!(
                "\"{name}\" is not a valid layout name (at most 255 characters, none of < > / \\ \" : ; ? * | , = `)."
            ));
        }
        if same_name(name, "Model") {
            return Err("\"Model\" is reserved for model space.".to_string());
        }
        if let Some(existing) = self.resolve_layout_name(i, name) {
            if renaming.is_none_or(|old| existing != old) {
                return Err(format!("a layout named \"{existing}\" already exists."));
            }
        }
        Ok(())
    }

    /// Paper unit used when a sheet size is written without one: inches in an
    /// imperial drawing, millimetres otherwise.
    fn default_paper_units(&self, i: usize) -> PaperUnits {
        let header = &self.tabs[i].scene.document.header;
        match header.insertion_units {
            1 | 2 | 3 | 8 | 9 | 10 | 21 => PaperUnits::Inches,
            0 if header.measurement == 0 => PaperUnits::Inches,
            _ => PaperUnits::Millimeters,
        }
    }

    fn layout_list(&mut self, i: usize) {
        let scene = &self.tabs[i].scene;
        let mut lines = vec![format!("Layouts (current: {}):", scene.current_layout)];
        for name in scene.layout_names().into_iter().skip(1) {
            let Some(layout) = find_layout(&scene.document, &name) else {
                continue;
            };
            let sheet = crate::scene::Scene::layout_sheet_viewport_handle(&scene.document, layout);
            let viewports = scene
                .document
                .entities()
                .filter(|entity| {
                    matches!(entity, EntityType::Viewport(viewport)
                        if viewport.common.owner_handle == layout.block_record
                            && viewport.common.handle != sheet)
                })
                .count();
            lines.push(format!(
                "  {}  {}  {}  {} viewport(s)",
                layout.tab_order,
                layout.name,
                sheet_label(layout),
                viewports
            ));
        }
        for line in lines {
            self.command_line.push_output(&line);
        }
    }

    fn layout_new(&mut self, i: usize, args: &[String]) {
        let Some(name) = args.first() else {
            self.command_line
                .push_error("LAYOUT N: a layout name is required.");
            return;
        };
        let name = name.trim().to_string();
        if let Err(error) = self.check_layout_name(i, &name, None) {
            self.command_line.push_error(&format!("LAYOUT N: {error}"));
            return;
        }
        let sheet = if args.len() > 1 {
            match parse_sheet(&args[1..], self.default_paper_units(i)) {
                Ok(sheet) => Some(sheet),
                Err(error) => {
                    self.command_line.push_error(&format!("LAYOUT N: {error}"));
                    return;
                }
            }
        } else {
            None
        };
        self.push_undo_snapshot(i, "LAYOUT NEW");
        crate::io::paper_space::normalize_paper_space_blocks(&mut self.tabs[i].scene.document, None);
        if let Err(error) = self.tabs[i].scene.document.add_layout(&name) {
            self.command_line.push_error(&format!("LAYOUT N: {error}"));
            return;
        }
        let flags = {
            let header = &self.tabs[i].scene.document.header;
            i16::from(header.paper_space_linetype_scaling)
                | (i16::from(header.paper_space_limit_check) << 1)
        };
        let plot_style = self
            .active_plot_style
            .as_ref()
            .map(|style| style.name.clone())
            .unwrap_or_default();
        let label = match find_layout_mut(&mut self.tabs[i].scene.document, &name) {
            Some(layout) => {
                layout.flags = flags;
                crate::scene::apply_default_page_setup(layout, &plot_style);
                if let Some(sheet) = &sheet {
                    apply_sheet(layout, sheet);
                }
                sheet_label(layout)
            }
            None => String::new(),
        };
        fit_sheet_viewport(&mut self.tabs[i].scene, &name);
        self.tabs[i].scene.bump_geometry();
        self.tabs[i].dirty = true;
        self.command_line
            .push_output(&format!("LAYOUT: \"{name}\" created ({label})."));
    }

    fn layout_set(&mut self, i: usize, args: &[String]) -> Task<Message> {
        let [name] = args else {
            self.command_line
                .push_error("LAYOUT S: exactly one layout name is required.");
            return Task::none();
        };
        let Some(target) = self.resolve_layout_name(i, name) else {
            self.command_line
                .push_error(&format!("LAYOUT S: no layout named \"{name}\"."));
            return Task::none();
        };
        let scene = &self.tabs[i].scene;
        if scene.current_layout == target && scene.active_viewport.is_none() {
            self.command_line
                .push_output(&format!("LAYOUT: \"{target}\" is already current."));
            return Task::none();
        }
        let task = self.on_layout_switch(target.clone());
        self.command_line
            .push_output(&format!("LAYOUT: current layout is \"{target}\"."));
        task
    }

    fn layout_rename(&mut self, i: usize, args: &[String]) {
        let [old, new] = args else {
            self.command_line
                .push_error("LAYOUT R: the current and the new name are required.");
            return;
        };
        let Some(old) = self
            .resolve_layout_name(i, old)
            .filter(|name| name != "Model")
        else {
            self.command_line
                .push_error(&format!("LAYOUT R: no paper layout named \"{old}\"."));
            return;
        };
        let new = new.trim().to_string();
        if let Err(error) = self.check_layout_name(i, &new, Some(old.as_str())) {
            self.command_line.push_error(&format!("LAYOUT R: {error}"));
            return;
        }
        if new == old {
            self.command_line
                .push_output(&format!("LAYOUT: \"{old}\" keeps its name."));
            return;
        }
        self.push_undo_snapshot(i, "LAYOUT RENAME");
        let scene = &mut self.tabs[i].scene;
        let is_current = scene.current_layout == old;
        if is_current {
            // The header holds the live sheet state of the current layout;
            // hand it to the layout object before the object changes name.
            scene.persist_current_layout_state();
        }
        scene.rename_layout(&old, &new);
        let dictionary_handle = scene.document.header.acad_layout_dict_handle;
        if let Some(ObjectType::Dictionary(dictionary)) =
            scene.document.objects.get_mut(&dictionary_handle)
        {
            for (key, _) in dictionary.entries.iter_mut() {
                if *key == old {
                    *key = new.clone();
                }
            }
        }
        if is_current {
            scene.set_current_layout(new.clone());
        }
        self.layout_rename_state = None;
        self.tabs[i].dirty = true;
        self.command_line
            .push_output(&format!("LAYOUT: \"{old}\" renamed \"{new}\"."));
    }

    fn layout_delete(&mut self, i: usize, args: &[String]) -> Task<Message> {
        let [name] = args else {
            self.command_line
                .push_error("LAYOUT D: exactly one layout name is required.");
            return Task::none();
        };
        let Some(name) = self.resolve_layout_name(i, name) else {
            self.command_line
                .push_error(&format!("LAYOUT D: no layout named \"{name}\"."));
            return Task::none();
        };
        if name == "Model" {
            self.command_line
                .push_error("LAYOUT D: the Model layout cannot be deleted.");
            return Task::none();
        }
        if self.tabs[i].scene.layout_names().len() <= 2 {
            self.command_line.push_error(&format!(
                "LAYOUT D: \"{name}\" is the only paper layout; a drawing keeps at least one."
            ));
            return Task::none();
        }
        let deleting_current = self.tabs[i].scene.current_layout == name;
        let cancel_task = if deleting_current {
            self.cancel_active_command_for_space_change()
        } else {
            Task::none()
        };
        self.push_undo_snapshot(i, "LAYOUT DELETE");
        let switch_task = if deleting_current {
            self.on_layout_switch("Model".to_string())
        } else {
            Task::none()
        };
        if self.tabs[i].scene.delete_layout(&name) {
            let scene = &mut self.tabs[i].scene;
            crate::io::paper_space::normalize_paper_space_blocks(&mut scene.document, None);
            let order: Vec<String> = scene.layout_names().into_iter().skip(1).collect();
            scene.set_layout_tab_order(&order);
            self.layout_rename_state = None;
            self.tabs[i].dirty = true;
            self.command_line
                .push_output(&format!("LAYOUT: \"{name}\" deleted."));
        } else {
            self.command_line
                .push_error(&format!("LAYOUT D: \"{name}\" could not be deleted."));
        }
        Task::batch([cancel_task, switch_task])
    }

    fn layout_paper(&mut self, i: usize, args: &[String]) {
        let Some(name) = args.first() else {
            self.command_line
                .push_error("LAYOUT P: a layout name and a sheet size are required.");
            return;
        };
        let Some(name) = self
            .resolve_layout_name(i, name)
            .filter(|name| name != "Model")
        else {
            self.command_line
                .push_error(&format!("LAYOUT P: no paper layout named \"{name}\"."));
            return;
        };
        let sheet = match parse_sheet(&args[1..], self.default_paper_units(i)) {
            Ok(sheet) => sheet,
            Err(error) => {
                self.command_line.push_error(&format!("LAYOUT P: {error}"));
                return;
            }
        };
        self.push_undo_snapshot(i, "LAYOUT PAPER");
        let scene = &mut self.tabs[i].scene;
        let is_current = scene.current_layout == name;
        if is_current {
            scene.persist_current_layout_state();
        }
        let label = match find_layout_mut(&mut scene.document, &name) {
            Some(layout) => {
                apply_sheet(layout, &sheet);
                sheet_label(layout)
            }
            None => String::new(),
        };
        fit_sheet_viewport(scene, &name);
        if is_current {
            // Back into the header, or leaving the layout would write the old
            // limits over the new sheet.
            scene.load_current_layout_state();
            scene.restore_saved_camera();
        }
        scene.bump_geometry();
        self.tabs[i].dirty = true;
        self.command_line
            .push_output(&format!("LAYOUT: \"{name}\" is now {label}."));
    }

    fn mview_one_line(&mut self, i: usize, args: &[String]) -> Task<Message> {
        use acadrust::types::Vector3;
        let spec = match parse_mview(args) {
            Ok(spec) => spec,
            Err(error) => {
                self.command_line.push_error(&error);
                return Task::none();
            }
        };
        if self.tabs[i].scene.active_viewport.is_some() {
            self.command_line
                .push_error("MVIEW: return to paper space first (PSPACE).");
            return Task::none();
        }
        let (x0, y0, x1, y1) = spec.rectangle();
        let view_height = spec.view_height();
        self.push_undo_snapshot(i, "MVIEW");
        if let Some(layer) = &spec.layer {
            self.tabs[i].scene.ensure_layer(layer);
        }
        let mut viewport = acadrust::entities::Viewport::new();
        viewport.center = Vector3::new((x0 + x1) / 2.0, (y0 + y1) / 2.0, 0.0);
        viewport.width = x1 - x0;
        viewport.height = y1 - y0;
        viewport.id = 2;
        // Committing places it in the current layout, gives it a unique id and
        // fits its view to the model extents; the line's own view follows.
        let Some(handle) = self.commit_entity_handle(EntityType::Viewport(viewport)) else {
            return Task::none();
        };
        let summary = match self.tabs[i].scene.document.get_entity_mut(handle) {
            Some(EntityType::Viewport(viewport)) => {
                if let Some((cx, cy)) = spec.center {
                    viewport.view_target = Vector3::new(cx, cy, 0.0);
                    viewport.view_center = Vector3::ZERO;
                }
                if let Some(height) = view_height {
                    viewport.view_height = height;
                }
                if viewport.view_height.abs() > 1e-12 {
                    viewport.custom_scale = viewport.height / viewport.view_height;
                }
                viewport.status.locked = spec.locked;
                if let Some(layer) = &spec.layer {
                    viewport.common.layer = layer.clone();
                }
                let scale = crate::scene::vp_effective_scale(
                    viewport.custom_scale,
                    viewport.view_height,
                    viewport.height,
                );
                format!(
                    "MVIEW: viewport {} created, {} x {} at {},{}, looking at {},{}, scale 1:{}{}.",
                    viewport.id,
                    format_number(viewport.width),
                    format_number(viewport.height),
                    format_number(viewport.center.x),
                    format_number(viewport.center.y),
                    format_number(viewport.view_target.x + viewport.view_center.x),
                    format_number(viewport.view_target.y + viewport.view_center.y),
                    if scale > 0.0 {
                        format_number(1.0 / scale)
                    } else {
                        "?".to_string()
                    },
                    if viewport.status.locked { ", locked" } else { "" }
                )
            }
            _ => String::new(),
        };
        self.tabs[i]
            .scene
            .bump_entities(&[(handle, crate::scene::ChangeKind::Modified)]);
        self.tabs[i].scene.camera_generation += 1;
        self.tabs[i].dirty = true;
        self.command_line.push_output(&summary);
        Task::none()
    }

    /// `LAYER PLOT <name> ON|OFF` — the Plot / No Plot switch of the Layers
    /// panel. A viewport border goes on a No Plot layer so the sheet prints
    /// without frames around its views.
    fn layer_plot_command(&mut self, i: usize, args: &[String]) -> Task<Message> {
        let [name, switch] = args else {
            self.command_line.push_error("Usage: LAYER PLOT <name> ON|OFF");
            return Task::none();
        };
        let plottable = match switch.to_uppercase().as_str() {
            "ON" | "YES" | "OUI" | "1" | "PLOT" => true,
            "OFF" | "NO" | "NON" | "0" | "NOPLOT" => false,
            _ => {
                self.command_line
                    .push_error(&format!("LAYER PLOT: \"{switch}\" is not ON or OFF."));
                return Task::none();
            }
        };
        let Some(stored) = self.tabs[i]
            .scene
            .document
            .layers
            .get(name)
            .map(|layer| layer.name.clone())
        else {
            self.command_line
                .push_error(&format!("LAYER PLOT: layer \"{name}\" not found."));
            return Task::none();
        };
        let targets = vec![stored.clone()];
        let undo = self.begin_layer_undo(i, "LAYER PLOT/NOPLOT", &targets);
        if let Some(layer) = self.tabs[i].scene.document.layers.get_mut(&stored) {
            layer.is_plottable = plottable;
        }
        if let Some(row) = self.tabs[i]
            .layers
            .layers
            .iter_mut()
            .find(|row| row.name == stored)
        {
            row.plottable = plottable;
        }
        self.tabs[i].layers.refresh_sort();
        self.tabs[i].scene.invalidate_layer_dependencies(&targets);
        self.tabs[i].dirty = true;
        self.commit_layer_undo(i, undo);
        self.command_line.push_output(&format!(
            "LAYER: \"{stored}\" set to {}.",
            if plottable { "Plot" } else { "No Plot" }
        ));
        Task::none()
    }

    /// `{"op":"layouts"}` — every layout with its sheet and floating viewports,
    /// for a headless caller to check what a script built.
    pub(crate) fn layouts_summary(&self) -> Value {
        let scene = &self.tabs[self.active_tab].scene;
        let document = &scene.document;
        let layouts: Vec<Value> = scene
            .layout_names()
            .into_iter()
            .map(|name| {
                if name == "Model" {
                    return json!({ "name": "Model", "tab_order": 0 });
                }
                let Some(layout) = find_layout(document, &name) else {
                    return json!({ "name": name });
                };
                let (min, max) = if scene.current_layout == name {
                    let header = &document.header;
                    (
                        (header.paper_space_limits_min.x, header.paper_space_limits_min.y),
                        (header.paper_space_limits_max.x, header.paper_space_limits_max.y),
                    )
                } else {
                    (layout.min_limits, layout.max_limits)
                };
                let sheet = crate::scene::Scene::layout_sheet_viewport_handle(document, layout);
                let sheet_viewport = sheet.is_valid().then(|| format!("{:X}", sheet.value()));
                let viewports: Vec<Value> = document
                    .entities()
                    .filter_map(|entity| match entity {
                        EntityType::Viewport(viewport)
                            if viewport.common.owner_handle == layout.block_record
                                && viewport.common.handle != sheet =>
                        {
                            Some(viewport_json(viewport))
                        }
                        _ => None,
                    })
                    .collect();
                json!({
                    "name": layout.name,
                    "tab_order": layout.tab_order,
                    "paper": {
                        "name": layout.paper_size,
                        "width": (max.0 - min.0).abs(),
                        "height": (max.1 - min.1).abs(),
                        "units": paper_units_label(layout.plot_paper_units),
                        "rotation": layout.plot_rotation,
                        "medium_mm": [layout.paper_width, layout.paper_height],
                    },
                    "limits": [[min.0, min.1], [max.0, max.1]],
                    "sheet_viewport": sheet_viewport,
                    "viewports": viewports,
                })
            })
            .collect();
        let model_space_viewport = scene
            .active_viewport
            .map(|handle| format!("{:X}", handle.value()));
        json!({
            "ok": true,
            "current": scene.current_layout,
            "model_space_viewport": model_space_viewport,
            "layouts": layouts,
        })
    }
}

// The command-line layout command answers to both spellings of the reference
// application; the dash form is the scripting one there.
inventory::submit!(crate::command::CommandRegistration {
    names: &["LAYOUT", "-LAYOUT"]
});

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn fresh_app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], true);
        app
    }

    fn run(app: &mut OpenCADStudio, cmd: &str) -> Value {
        app.automation_op(&json!({ "op": "run", "cmd": cmd }).to_string())
    }

    fn run_ok(app: &mut OpenCADStudio, cmd: &str) -> Value {
        let response = run(app, cmd);
        assert_eq!(response["ok"], true, "{cmd}: {response}");
        response
    }

    fn run_refused(app: &mut OpenCADStudio, cmd: &str, reason: &str) {
        let before = app.automation_op(r#"{"op":"entities"}"#)["total"].clone();
        let response = run(app, cmd);
        assert_eq!(response["ok"], false, "{cmd} should be refused: {response}");
        let error = response["error"].as_str().unwrap_or_default();
        assert!(error.contains(reason), "{cmd}: \"{error}\" lacks \"{reason}\"");
        assert_eq!(
            app.automation_op(r#"{"op":"entities"}"#)["total"],
            before,
            "{cmd} must not change the drawing"
        );
    }

    /// The current layout's sheet, as the engine itself sizes it from the
    /// stored paper (millimetres through the paper-unit factor).
    fn assert_sheet(app: &OpenCADStudio, width: f64, height: f64) {
        let ((x0, y0), (x1, y1)) = app.tabs[app.active_tab]
            .scene
            .paper_limits()
            .expect("paper limits");
        assert!(
            close(x0, 0.0) && close(y0, 0.0) && close(x1, width) && close(y1, height),
            "sheet {x0},{y0} {x1},{y1} is not {width} x {height}"
        );
    }

    fn layout<'a>(app: &'a OpenCADStudio, name: &str) -> &'a Layout {
        find_layout(&app.tabs[app.active_tab].scene.document, name)
            .unwrap_or_else(|| panic!("layout {name}"))
    }

    fn content_viewports(app: &OpenCADStudio, name: &str) -> Vec<acadrust::entities::Viewport> {
        let document = &app.tabs[app.active_tab].scene.document;
        let layout = find_layout(document, name).expect("layout");
        let sheet = crate::scene::Scene::layout_sheet_viewport_handle(document, layout);
        let mut viewports: Vec<_> = document
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Viewport(viewport)
                    if viewport.common.owner_handle == layout.block_record
                        && viewport.common.handle != sheet =>
                {
                    Some(viewport.clone())
                }
                _ => None,
            })
            .collect();
        viewports.sort_by_key(|viewport| viewport.id);
        viewports
    }

    #[test]
    fn quoted_names_stay_whole_and_lengths_keep_their_marks() {
        assert_eq!(
            split_args(r#"N "Plan RDC" 36 24 IN"#).unwrap(),
            vec!["N", "Plan RDC", "36", "24", "IN"]
        );
        assert_eq!(split_args("  S   A-101 ").unwrap(), vec!["S", "A-101"]);
        assert_eq!(
            split_args(r#"SCALE 1/4"=1'-0" LOCK"#).unwrap(),
            vec!["SCALE", r#"1/4"=1'-0""#, "LOCK"]
        );
        assert_eq!(split_args(r#"R "" X"#).unwrap(), vec!["R", "", "X"]);
        assert!(split_args(r#"N "Plan RDC 36"#).is_err());
        assert!(split_args(r#"N "Plan"RDC"#).is_err());
        assert!(split_args("").unwrap().is_empty());
    }

    #[test]
    fn scales_read_every_usual_spelling() {
        let scale = |text: &str| parse_scale(text).unwrap_or(f64::NAN);
        assert!(close(scale("1/48"), 1.0 / 48.0));
        assert!(close(scale("1:48"), 1.0 / 48.0));
        assert!(close(scale("1/48XP"), 1.0 / 48.0));
        assert!(close(scale("1/48xp"), 1.0 / 48.0));
        assert!(close(scale(r#"1/4"=1'-0""#), 1.0 / 48.0));
        assert!(close(scale(r#"3/16"=1'"#), 1.0 / 64.0));
        assert!(close(scale("1=100"), 0.01));
        assert!(close(scale("0.5"), 0.5));
        for bad in ["", "0", "-1/48", "1:0", "abc", "1/0", "1=0"] {
            assert_eq!(parse_scale(bad), None, "{bad}");
        }
    }

    #[test]
    fn sheets_resolve_to_catalogue_media() {
        let arch_d = parse_sheet(&tokens(&["36", "24", "IN"]), PaperUnits::Millimeters).unwrap();
        assert_eq!(arch_d.paper.canonical, "ARCH_D_(24.00_x_36.00_Inches)");
        assert!(arch_d.landscape);
        assert_eq!(arch_d.units, PaperUnits::Inches);
        assert_eq!(arch_d.size(), (36.0, 24.0));

        let a4 = parse_sheet(&tokens(&["210", "297", "mm"]), PaperUnits::Inches).unwrap();
        assert_eq!(a4.paper.canonical, "ISO_A4_(210.00_x_297.00_MM)");
        assert!(!a4.landscape);
        assert_eq!(a4.size(), (210.0, 297.0));

        // Default unit, and a size no catalogue sheet has.
        let custom = parse_sheet(&tokens(&["30", "20"]), PaperUnits::Inches).unwrap();
        assert_eq!(custom.paper.canonical, "UserDefinedInches_(20.00_x_30.00_Inches)");
        assert_eq!(custom.size(), (30.0, 20.0));

        let a3 = parse_sheet(&tokens(&["A3"]), PaperUnits::Inches).unwrap();
        assert!(a3.landscape);
        assert_eq!(a3.units, PaperUnits::Millimeters);
        assert_eq!(a3.size(), (420.0, 297.0));
        let letter = parse_sheet(&tokens(&["Letter", "PORTRAIT"]), PaperUnits::Millimeters).unwrap();
        assert!(!letter.landscape);
        assert_eq!(letter.size(), (8.5, 11.0));

        for bad in [
            vec!["36", "24", "FT"],
            vec!["36"],
            vec!["0", "24"],
            vec!["36", "24", "IN", "X"],
            vec!["Letter", "SIDEWAYS"],
        ] {
            assert!(parse_sheet(&tokens(&bad), PaperUnits::Inches).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn mview_line_parses_every_option() {
        let spec = parse_mview(&tokens(&[
            "32,23", "1,1", "CENTER", "745.2,552", "SCALE", "1/48", "LOCK", "LAYER", "FENETRES-PRES",
        ]))
        .unwrap();
        assert_eq!(spec.rectangle(), (1.0, 1.0, 32.0, 23.0));
        assert_eq!(spec.center, Some((745.2, 552.0)));
        assert!(close(spec.view_height().unwrap(), 22.0 * 48.0));
        assert!(spec.locked);
        assert_eq!(spec.layer.as_deref(), Some("FENETRES-PRES"));

        let french = parse_mview(&tokens(&[
            "0,0", "10,8", "CENTRE", "5'-0\",4'", "HAUTEUR", "8'", "VERROU", "OFF", "CALQUE", "FEN",
        ]))
        .unwrap();
        assert_eq!(french.center, Some((60.0, 48.0)));
        assert!(close(french.view_height().unwrap(), 96.0));
        assert!(!french.locked);

        let fit = parse_mview(&tokens(&["1,1", "2,2"])).unwrap();
        assert_eq!(fit.view_height(), None);
        assert_eq!(fit.center, None);

        for (bad, reason) in [
            (vec!["1,1"], "two paper corners"),
            (vec!["1,1", "2,2", "SCALE", "1/48", "HEIGHT", "10"], "not both"),
            (vec!["1,1", "1,5", "SCALE", "1/48"], "area"),
            (vec!["1,1", "2,2", "FOO"], "unknown option"),
            (vec!["1,1", "2,2", "SCALE"], "needs a value"),
            (vec!["1,1", "2,2", "SCALE", "0"], "not a scale"),
            (vec!["1,1", "2,2", "CENTER", "@5,5"], "not a model point"),
            (vec!["1,1", "2,2", "LAYER", "A|B"], "not a valid layer"),
        ] {
            let error = parse_mview(&tokens(&bad)).unwrap_err();
            assert!(error.contains(reason), "{bad:?}: {error}");
        }
    }

    #[test]
    fn layout_command_names_sizes_switches_and_deletes_without_a_window() {
        let mut app = fresh_app();
        let i = app.active_tab;

        run_ok(&mut app, "LAYOUT N A-101 36 24 IN");
        // Creating does not switch, as in the reference command.
        assert_eq!(app.tabs[i].scene.current_layout, "Model");
        let created = layout(&app, "A-101");
        assert_eq!(created.paper_size, "ARCH_D_(24.00_x_36.00_Inches)");
        assert!(close(created.paper_width, 609.6) && close(created.paper_height, 914.4));
        assert_eq!(created.plot_rotation, 1);
        assert_eq!(created.plot_paper_units, 0);
        assert_eq!(created.min_limits, (0.0, 0.0));
        assert_eq!(created.max_limits, (36.0, 24.0));
        // The overall viewport frames the sheet.
        let document = &app.tabs[i].scene.document;
        let sheet = crate::scene::Scene::layout_sheet_viewport_handle(document, created);
        let Some(EntityType::Viewport(sheet)) = document.get_entity(sheet) else {
            panic!("sheet viewport");
        };
        assert_eq!((sheet.center.x, sheet.center.y), (18.0, 12.0));
        assert_eq!((sheet.width, sheet.height), (36.0, 24.0));
        // The engine's own paper unit factor reads the sheet in inches.
        run_ok(&mut app, "LAYOUT S a-101");
        assert_eq!(app.tabs[i].scene.current_layout, "A-101");
        assert_sheet(&app, 36.0, 24.0);

        // A new sheet on the current layout survives leaving it.
        run_ok(&mut app, "LAYOUT P A-101 11 17 IN");
        assert_eq!(layout(&app, "A-101").paper_size, "ANSI_B_(11.00_x_17.00_Inches)");
        assert_eq!(layout(&app, "A-101").plot_rotation, 0);
        run_ok(&mut app, "LAYOUT S Model");
        assert_eq!(app.tabs[i].scene.current_layout, "Model");
        assert_eq!(layout(&app, "A-101").max_limits, (11.0, 17.0));
        run_ok(&mut app, "LAYOUT S A-101");
        assert_sheet(&app, 11.0, 17.0);

        // Renaming the current layout follows it, dictionary key included.
        run_ok(&mut app, r#"LAYOUT R A-101 "Plan RDC""#);
        assert_eq!(app.tabs[i].scene.current_layout, "Plan RDC");
        let document = &app.tabs[i].scene.document;
        let Some(ObjectType::Dictionary(dictionary)) =
            document.objects.get(&document.header.acad_layout_dict_handle)
        else {
            panic!("layout dictionary");
        };
        assert!(dictionary.get("Plan RDC").is_some());
        assert!(dictionary.get("A-101").is_none());

        // Deleting the layout that owns *Paper_Space hands that block on.
        run_ok(&mut app, "LAYOUT D Layout1");
        assert_eq!(app.tabs[i].scene.layout_names(), vec!["Model", "Plan RDC"]);
        let document = &app.tabs[i].scene.document;
        let primary = document
            .block_records
            .get("*Paper_Space")
            .expect("a *Paper_Space block remains");
        assert_eq!(primary.handle, layout(&app, "Plan RDC").block_record);
        assert_eq!(document.header.paper_space_block_handle, primary.handle);
        assert_eq!(layout(&app, "Plan RDC").tab_order, 1);

        // The next layout still gets a block of its own.
        run_ok(&mut app, "LAYOUT N A-201 A3");
        run_ok(&mut app, "LAYOUT N A-301 36 24 IN");
        run_ok(&mut app, "LAYOUT D A-201");
        run_ok(&mut app, "LAYOUT N A-401 36 24 IN");
        assert_eq!(
            app.tabs[i].scene.layout_names(),
            vec!["Model", "Plan RDC", "A-301", "A-401"]
        );

        run_refused(&mut app, r#"LAYOUT D "Plan RDC" extra"#, "exactly one");
        run_refused(&mut app, "LAYOUT D Model", "cannot be deleted");
        run_refused(&mut app, "LAYOUT N a-301", "already exists");
        run_refused(&mut app, "LAYOUT N Model", "reserved");
        run_refused(&mut app, "LAYOUT N A<1", "not a valid layout name");
        run_refused(&mut app, "LAYOUT N A-501 36 24 FT", "unknown paper unit");
        run_refused(&mut app, "LAYOUT X", "unknown option");
        run_refused(&mut app, "LAYOUT S Nowhere", "no layout named");
        run_refused(&mut app, "LAYOUT P Model 36 24 IN", "no paper layout");
        run_refused(&mut app, r#"LAYOUT N "open"#, "not closed");
        run_ok(&mut app, "LAYOUT D A-301");
        run_ok(&mut app, "LAYOUT D A-401");
        run_refused(&mut app, r#"LAYOUT D "Plan RDC""#, "only paper layout");
        run_ok(&mut app, "LAYOUT S Model");
        assert_eq!(app.tabs[i].scene.current_layout, "Model");
    }

    #[test]
    fn mview_line_sets_centre_scale_lock_and_layer() {
        let mut app = fresh_app();
        let i = app.active_tab;
        run_ok(&mut app, "RECTANG 0,0 480,288");
        run_ok(&mut app, "LAYOUT N A-101 36 24 IN");

        // Model space refuses with a message that says what to do.
        run_refused(&mut app, "MVIEW 1,1 32,23 SCALE 1/48", "paper space layout first");
        run_refused(&mut app, "MVIEW F", "paper space layout first");

        run_ok(&mut app, "LAYOUT S A-101");
        let response = run_ok(
            &mut app,
            "MVIEW 1,1 32,23 CENTER 745.2,552 SCALE 1/48 LOCK LAYER FENETRES-PRES",
        );
        assert_eq!(response["added"], 1, "{response}");
        let viewports = content_viewports(&app, "A-101");
        assert_eq!(viewports.len(), 1);
        let viewport = &viewports[0];
        assert!(viewport.id >= 2);
        assert_eq!((viewport.center.x, viewport.center.y), (16.5, 12.0));
        assert_eq!((viewport.width, viewport.height), (31.0, 22.0));
        assert_eq!(
            (viewport.view_target.x, viewport.view_target.y, viewport.view_target.z),
            (745.2, 552.0, 0.0)
        );
        assert_eq!((viewport.view_center.x, viewport.view_center.y), (0.0, 0.0));
        assert!(close(viewport.view_height, 22.0 * 48.0));
        assert!(close(
            crate::scene::vp_effective_scale(viewport.custom_scale, viewport.view_height, viewport.height),
            1.0 / 48.0
        ));
        assert!(viewport.status.locked);
        assert!(viewport.status.is_on);
        assert_eq!(viewport.common.layer, "FENETRES-PRES");
        assert!(app.tabs[i].scene.document.layers.contains("FENETRES-PRES"));

        // HEIGHT instead of SCALE, French keywords, and a quoted layer name.
        run_ok(
            &mut app,
            r#"MVIEW 20,2 35,10 CENTRE 100,100 HAUTEUR 8' CALQUE "FEN PRES""#,
        );
        let viewports = content_viewports(&app, "A-101");
        assert_eq!(viewports.len(), 2);
        let second = &viewports[1];
        assert!(close(second.view_height, 96.0));
        assert!(!second.status.locked);
        assert_eq!(second.common.layer, "FEN PRES");
        assert_ne!(second.id, viewports[0].id);

        // Two corners alone keep the interactive result: fit to the model.
        run_ok(&mut app, "MVIEW 1,14 10,22");
        let viewports = content_viewports(&app, "A-101");
        assert_eq!(viewports.len(), 3);
        let fitted = &viewports[2];
        assert_eq!((fitted.view_target.x, fitted.view_target.y), (240.0, 144.0));

        run_refused(&mut app, "MVIEW 1,1 2,2 SCALE 1/48 HEIGHT 10", "not both");
        run_refused(&mut app, "MVIEW 1,1 1,5 SCALE 1/48", "area");
        run_refused(&mut app, "MVIEW 1,1 2,2 ZOOM 3", "unknown option");
        assert_eq!(content_viewports(&app, "A-101").len(), 3);
    }

    #[test]
    fn layer_plot_switch_is_typeable() {
        let mut app = fresh_app();
        let i = app.active_tab;
        run_ok(&mut app, "LAYER NEW FENETRES-PRES");
        run_ok(&mut app, "LAYER PLOT FENETRES-PRES OFF");
        assert!(!app.tabs[i].scene.document.layers.get("FENETRES-PRES").unwrap().is_plottable);
        run_ok(&mut app, "LAYER PLOT fenetres-pres ON");
        assert!(app.tabs[i].scene.document.layers.get("FENETRES-PRES").unwrap().is_plottable);
        run_refused(&mut app, "LAYER PLOT NOPE OFF", "not found");
        run_refused(&mut app, "LAYER PLOT FENETRES-PRES MAYBE", "not ON or OFF");
    }

    /// The whole set of a drawing — two ARCH D sheets, viewports at 1:48 and
    /// 1:8 on a No Plot layer, a frame drawn on paper — written to DWG through
    /// the automation `save` and read back through `open`.
    #[test]
    fn a_sheet_set_survives_a_dwg_round_trip() {
        let path = std::env::temp_dir().join(format!("ocs_p4_layouts_{}.dwg", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut app = fresh_app();
        for cmd in [
            "RECTANG 0,0 1490.4,1104",
            "CIRCLE 100,100 20",
            "LAYOUT R Layout1 A-001",
            "LAYOUT P A-001 36 24 IN",
            "LAYOUT N A-101 36 24 IN",
            "LAYER NEW FENETRES-PRES",
            "LAYER PLOT FENETRES-PRES OFF",
            "LAYOUT S A-101",
            "MVIEW 0.5,1 31.55,24 CENTER 745.2,552 SCALE 1/48 LOCK LAYER FENETRES-PRES",
            "MVIEW 20,2 35,10 CENTER 100,100 SCALE 1/8 LOCK LAYER FENETRES-PRES",
            "RECTANG 0.5,0.5 35.5,23.5",
            "LAYOUT S Model",
        ] {
            run_ok(&mut app, cmd);
        }
        let p = path.to_string_lossy().replace('\\', "\\\\");
        let saved = app.automation_op(&format!(r#"{{"op":"save","path":"{p}"}}"#));
        assert_eq!(saved["ok"], true, "{saved}");
        drop(app);

        let mut app = fresh_app();
        let opened = app.automation_op(&format!(r#"{{"op":"open","path":"{p}"}}"#));
        assert_eq!(opened["ok"], true, "{opened}");
        let summary = app.automation_op(r#"{"op":"layouts"}"#);
        assert_eq!(summary["ok"], true, "{summary}");
        let names: Vec<&str> = summary["layouts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|layout| layout["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, vec!["Model", "A-001", "A-101"], "{summary}");
        let sheet = &summary["layouts"][2];
        assert_eq!(sheet["paper"]["name"], "ARCH_D_(24.00_x_36.00_Inches)");
        assert_eq!(sheet["paper"]["units"], "in");
        assert_eq!(sheet["paper"]["rotation"], 1);
        assert_eq!(sheet["limits"], json!([[0.0, 0.0], [36.0, 24.0]]));
        let viewports = sheet["viewports"].as_array().unwrap();
        assert_eq!(viewports.len(), 2, "{sheet}");
        let mut found = [false, false];
        for viewport in viewports {
            assert_eq!(viewport["layer"], "FENETRES-PRES");
            assert_eq!(viewport["locked"], true);
            let factor = viewport["scale_factor"].as_f64().unwrap();
            if close(factor, 48.0) {
                found[0] = true;
                assert!(close(viewport["center"][0].as_f64().unwrap(), 16.025));
                assert!(close(viewport["center"][1].as_f64().unwrap(), 12.5));
                assert!(close(viewport["width"].as_f64().unwrap(), 31.05));
                assert!(close(viewport["view_center"][0].as_f64().unwrap(), 745.2));
                assert!(close(viewport["view_center"][1].as_f64().unwrap(), 552.0));
                assert!(close(viewport["view_height"].as_f64().unwrap(), 23.0 * 48.0));
            } else if close(factor, 8.0) {
                found[1] = true;
                assert!(close(viewport["view_height"].as_f64().unwrap(), 64.0));
            }
        }
        assert_eq!(found, [true, true], "{sheet}");
        let document = &app.tabs[app.active_tab].scene.document;
        assert!(!document.layers.get("FENETRES-PRES").unwrap().is_plottable);
        // The paper frame stayed on the sheet it was drawn on.
        let a101 = find_layout(document, "A-101").unwrap();
        assert_eq!(
            document
                .entities()
                .filter(|entity| matches!(entity, EntityType::LwPolyline(_))
                    && entity.common().owner_handle == a101.block_record)
                .count(),
            1
        );
        // Saved in Model space, A-001 kept *Paper_Space: its entities were
        // written in paper mode (1), A-101's as owned (0) — as the reference
        // application writes them, and what a plain ODA read needs to keep
        // the A-101 viewports active (see `io/paper_space.rs`).
        let a001 = find_layout(document, "A-001").unwrap();
        let modes = |block: acadrust::Handle| -> Vec<Option<u8>> {
            document
                .entities()
                .filter(|entity| entity.common().owner_handle == block)
                .map(|entity| entity.common().entity_mode)
                .collect()
        };
        let secondary = modes(a101.block_record);
        assert_eq!(secondary.len(), 4, "{secondary:?}");
        assert!(secondary.iter().all(|mode| *mode == Some(0)), "{secondary:?}");
        let primary = modes(a001.block_record);
        assert!(!primary.is_empty());
        assert!(primary.iter().all(|mode| *mode == Some(1)), "{primary:?}");
        drop(app);
        let sidecar = path.with_file_name(format!(
            ".{}.ocs.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_file(sidecar);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn layouts_op_reports_sheets_and_viewports() {
        let mut app = fresh_app();
        run_ok(&mut app, "LAYOUT N A-101 36 24 IN");
        run_ok(&mut app, "LAYOUT S A-101");
        run_ok(&mut app, "MVIEW 1,1 32,23 CENTER 0,0 HEIGHT 1056 LOCK");
        let summary = app.automation_op(r#"{"op":"layouts"}"#);
        assert_eq!(summary["current"], "A-101");
        let layouts = summary["layouts"].as_array().unwrap();
        let a101 = layouts.iter().find(|layout| layout["name"] == "A-101").unwrap();
        assert_eq!(a101["paper"]["width"], 36.0);
        assert_eq!(a101["paper"]["height"], 24.0);
        assert_eq!(a101["viewports"].as_array().unwrap().len(), 1);
        let viewport = &a101["viewports"][0];
        assert!(close(viewport["scale_factor"].as_f64().unwrap(), 48.0));
        assert_eq!(viewport["view_center"], json!([0.0, 0.0]));
        assert_eq!(viewport["locked"], true);
    }

    /// The groups of every VIEWPORT of an ASCII DXF: handle (5), owner
    /// (330), status (68), id (69), flags (90).
    fn dxf_viewports(path: &std::path::Path) -> Vec<std::collections::HashMap<i32, String>> {
        let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        let mut viewports = Vec::new();
        let mut current: Option<std::collections::HashMap<i32, String>> = None;
        for pair in lines.chunks(2) {
            let [code, value] = pair else { break };
            let Ok(code) = code.trim().parse::<i32>() else { continue };
            if code == 0 {
                viewports.extend(current.take());
                if value.trim() == "VIEWPORT" {
                    current = Some(Default::default());
                }
            } else if let Some(groups) = current.as_mut() {
                if matches!(code, 5 | 330 | 68 | 69 | 90) {
                    groups.entry(code).or_insert_with(|| value.trim().to_string());
                }
            }
        }
        viewports.extend(current);
        viewports
    }

    /// The set of the reference sheets (A-501, escalier) — seven ARCH D
    /// layouts, 29 locked viewports — saved to DWG, reopened and written to
    /// DXF, the file the server previews and turns into PDF. Every viewport
    /// is on (status > 0, 0x8000, never 0x20000) with its id on its sheet:
    /// 1 for the sheet, then 2, 3, … Before, the DXF had no group 68 and id 0
    /// everywhere, and ezdxf drew every sheet empty.
    #[test]
    fn a_sheet_set_written_to_dxf_keeps_its_viewports_on() {
        let stem = format!("ocs_p4_dxf_viewports_{}", std::process::id());
        let dwg = std::env::temp_dir().join(format!("{stem}.dwg"));
        let dxf = std::env::temp_dir().join(format!("{stem}.dxf"));
        let sheets = [
            ("A-001", 2),
            ("A-100", 3),
            ("A-101", 3),
            ("A-102", 3),
            ("A-301", 3),
            ("A-302", 3),
            ("A-501", 12),
        ];
        let mut app = fresh_app();
        run_ok(&mut app, "CIRCLE 0,0 10");
        run_ok(&mut app, "LAYOUT R Layout1 A-001");
        run_ok(&mut app, "LAYOUT P A-001 36 24 IN");
        for (name, _) in &sheets[1..] {
            run_ok(&mut app, &format!("LAYOUT N {name} 36 24 IN"));
        }
        for (name, count) in sheets {
            run_ok(&mut app, &format!("LAYOUT S {name}"));
            for n in 0..count {
                let x = 1.0 + 2.8 * n as f64;
                run_ok(
                    &mut app,
                    &format!("MVIEW {x},1 {},3 CENTER 0,0 SCALE 1/8 LOCK", x + 2.5),
                );
            }
        }
        run_ok(&mut app, "LAYOUT S Model");
        let path = |p: &std::path::Path| p.to_string_lossy().replace('\\', "\\\\");
        let saved = app.automation_op(&format!(r#"{{"op":"save","path":"{}"}}"#, path(&dwg)));
        assert_eq!(saved["ok"], true, "{saved}");
        drop(app);

        let mut app = fresh_app();
        let opened = app.automation_op(&format!(r#"{{"op":"open","path":"{}"}}"#, path(&dwg)));
        assert_eq!(opened["ok"], true, "{opened}");
        let saved = app.automation_op(&format!(r#"{{"op":"save","path":"{}"}}"#, path(&dxf)));
        assert_eq!(saved["ok"], true, "{saved}");

        let viewports = dxf_viewports(&dxf);
        assert_eq!(viewports.len(), 36, "7 sheet viewports and 29 views");
        let mut by_sheet: std::collections::HashMap<String, Vec<i32>> = Default::default();
        let mut views = 0;
        for groups in &viewports {
            let number = |code: i32| -> i32 {
                groups
                    .get(&code)
                    .unwrap_or_else(|| panic!("group {code} missing: {groups:?}"))
                    .parse()
                    .unwrap()
            };
            let (status, id, flags) = (number(68), number(69), number(90));
            assert!(status > 0, "on: {groups:?}");
            assert_eq!(status, id, "stacked in id order: {groups:?}");
            assert_ne!(flags & 0x8000, 0, "0x8000 always set: {groups:?}");
            assert_eq!(flags & 0x2_0000, 0, "never 0x20000 when on: {groups:?}");
            if id != 1 {
                views += 1;
                assert_ne!(flags & 0x4000, 0, "locked: {groups:?}");
            }
            by_sheet.entry(groups[&330].clone()).or_default().push(id);
        }
        assert_eq!(views, 29);
        assert_eq!(by_sheet.len(), 7);
        let mut counts: Vec<usize> = by_sheet.values().map(Vec::len).collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![3, 4, 4, 4, 4, 4, 13]);
        for ids in by_sheet.values_mut() {
            ids.sort_unstable();
            assert_eq!(*ids, (1..=ids.len() as i32).collect::<Vec<_>>());
        }

        // The engine reads its own DXF back with every view on and locked.
        drop(app);
        let mut app = fresh_app();
        let opened = app.automation_op(&format!(r#"{{"op":"open","path":"{}"}}"#, path(&dxf)));
        assert_eq!(opened["ok"], true, "{opened}");
        let summary = app.automation_op(r#"{"op":"layouts"}"#);
        let listed: Vec<&Value> = summary["layouts"]
            .as_array()
            .unwrap()
            .iter()
            // Model has no viewports list.
            .flat_map(|layout| layout["viewports"].as_array().into_iter().flatten())
            .collect();
        assert_eq!(listed.len(), 29, "{summary}");
        assert!(listed.iter().all(|v| v["on"] == true && v["locked"] == true), "{summary}");
        drop(app);
        for file in [&dwg, &dxf] {
            let sidecar = file.with_file_name(format!(
                ".{}.ocs.lock",
                file.file_name().unwrap().to_string_lossy()
            ));
            let _ = std::fs::remove_file(sidecar);
            let _ = std::fs::remove_file(file);
        }
    }
}
