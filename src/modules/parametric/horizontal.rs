use acadrust::{EntityType, Handle};
use glam::DVec3;

use crate::command::{
    CadCommand, CmdOption, CmdResult, CoincidentPick, HorizontalConstraintSelection, WorkingPlane,
};
use crate::scene::parametric_constraints::{
    directional_axis_endpoints, is_parametric_point_near, ParametricRef,
};

#[derive(Clone, Copy)]
enum Step {
    ObjectOrTwoPoints,
    FirstPoint,
    SecondPoint(CoincidentPick),
}

/// Reference-compatible front end for the Horizontal geometric constraint.
/// It deliberately uses entity picks rather than the generic selection set so
/// a polyline segment or one ellipse axis can be addressed unambiguously.
pub struct HorizontalConstraintCommand {
    step: Step,
    picked_entity: Option<EntityType>,
    plane: WorkingPlane,
}

impl HorizontalConstraintCommand {
    pub fn new() -> Self {
        Self {
            step: Step::ObjectOrTwoPoints,
            picked_entity: None,
            plane: WorkingPlane::default(),
        }
    }

    fn point_pick(handle: Option<Handle>, point: DVec3) -> CoincidentPick {
        CoincidentPick {
            handle,
            point,
            whole_curve: false,
        }
    }

    fn direction(&self) -> acadrust::types::Vector3 {
        let direction = self.plane.x.normalize_or(DVec3::X);
        acadrust::types::Vector3::new(direction.x, direction.y, direction.z)
    }

    fn invalid_object() -> CmdResult {
        CmdResult::ReportError(
            "Horizontal: select a line, straight polyline segment, text, MText, or an ellipse axis."
                .to_string(),
        )
    }

    fn invalid_point() -> CmdResult {
        CmdResult::ReportError(
            "Horizontal: select an endpoint, center, midpoint, or polyline vertex."
                .to_string(),
        )
    }

    fn segment_is_straight(entity: &EntityType, index: usize) -> bool {
        match entity {
            EntityType::LwPolyline(polyline) => polyline
                .vertices
                .get(index)
                .is_some_and(|vertex| vertex.bulge.abs() <= 1.0e-12),
            EntityType::Polyline2D(polyline) => polyline
                .vertices
                .get(index)
                .is_some_and(|vertex| vertex.bulge.abs() <= 1.0e-12),
            _ => false,
        }
    }

    fn distance_to_axis(point: DVec3, endpoints: [acadrust::types::Vector3; 2]) -> f64 {
        let start = DVec3::new(endpoints[0].x, endpoints[0].y, endpoints[0].z);
        let end = DVec3::new(endpoints[1].x, endpoints[1].y, endpoints[1].z);
        let direction = end - start;
        if direction.length_squared() <= 1.0e-24 {
            return f64::INFINITY;
        }
        (point - start).cross(direction.normalize()).length()
    }

    fn ellipse_reference(entity: &EntityType, handle: Handle, point: DVec3) -> Option<ParametricRef> {
        let major = ParametricRef::ellipse_major_axis(handle);
        let minor = ParametricRef::ellipse_minor_axis(handle);
        let major_distance =
            Self::distance_to_axis(point, directional_axis_endpoints(entity, major)?);
        let minor_distance =
            Self::distance_to_axis(point, directional_axis_endpoints(entity, minor)?);
        Some(if minor_distance < major_distance { minor } else { major })
    }

    fn picked_reference(entity: &EntityType, handle: Handle, point: DVec3) -> Option<ParametricRef> {
        match entity {
            EntityType::Line(_) | EntityType::Ray(_) | EntityType::XLine(_) => {
                Some(ParametricRef::whole(handle))
            }
            EntityType::LwPolyline(_) | EntityType::Polyline2D(_) => {
                let (source, _, _) = crate::scene::centerline::picked_source(entity, handle, point)?;
                let index = usize::try_from(source.segment_index).ok()?;
                Self::segment_is_straight(entity, index)
                    .then_some(ParametricRef::segment(handle, index))
            }
            EntityType::Text(_) | EntityType::MText(_) => {
                Some(ParametricRef::text_baseline(handle))
            }
            EntityType::Ellipse(_) => Self::ellipse_reference(entity, handle, point),
            _ => None,
        }
    }

    pub fn preselected_reference(entity: &EntityType, handle: Handle) -> Option<ParametricRef> {
        match entity {
            EntityType::Line(_) | EntityType::Ray(_) | EntityType::XLine(_) => {
                Some(ParametricRef::whole(handle))
            }
            EntityType::Text(_) | EntityType::MText(_) => {
                Some(ParametricRef::text_baseline(handle))
            }
            EntityType::Ellipse(_) => Some(ParametricRef::ellipse_major_axis(handle)),
            EntityType::LwPolyline(polyline) => {
                let count = if polyline.is_closed {
                    polyline.vertices.len()
                } else {
                    polyline.vertices.len().saturating_sub(1)
                };
                (count == 1 && Self::segment_is_straight(entity, 0))
                    .then_some(ParametricRef::segment(handle, 0))
            }
            EntityType::Polyline2D(polyline) => {
                let count = if polyline.is_closed() {
                    polyline.vertices.len()
                } else {
                    polyline.vertices.len().saturating_sub(1)
                };
                (count == 1 && Self::segment_is_straight(entity, 0))
                    .then_some(ParametricRef::segment(handle, 0))
            }
            _ => None,
        }
    }

    fn finish_points(&self, first: CoincidentPick, second: CoincidentPick) -> CmdResult {
        CmdResult::AddHorizontalConstraint {
            selection: HorizontalConstraintSelection::Points(first, second),
            direction: self.direction(),
            label: "Horizontal constraint",
        }
    }
}

impl CadCommand for HorizontalConstraintCommand {
    fn name(&self) -> &'static str {
        "GCHORIZONTAL"
    }

    fn prompt(&self) -> String {
        match self.step {
            Step::ObjectOrTwoPoints => {
                "GCHORIZONTAL  Select an object or [2Points] <2Points>:".to_string()
            }
            Step::FirstPoint => "GCHORIZONTAL  Select first point:".to_string(),
            Step::SecondPoint(_) => "GCHORIZONTAL  Select second point:".to_string(),
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        matches!(self.step, Step::ObjectOrTwoPoints)
            .then(|| vec![CmdOption::new("2Points", "2P")])
            .unwrap_or_default()
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn point_step_accepts_keywords(&self) -> bool {
        matches!(self.step, Step::ObjectOrTwoPoints)
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let keyword = text.trim().trim_start_matches('_').to_ascii_uppercase();
        if matches!(self.step, Step::ObjectOrTwoPoints)
            && matches!(keyword.as_str(), "2" | "2P" | "2POINT" | "2POINTS")
        {
            self.step = Step::FirstPoint;
            Some(CmdResult::NeedPoint)
        } else {
            None
        }
    }

    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn needs_entity_pick(&self) -> bool {
        true
    }

    fn entity_pick_accepts_points(&self) -> bool {
        true
    }

    fn entity_pick_highlights_hover(&self) -> bool {
        true
    }

    fn inject_before_entity_pick(&self) -> bool {
        true
    }

    fn inject_picked_entity(&mut self, entity: EntityType) {
        self.picked_entity = Some(entity);
    }

    fn on_entity_pick(&mut self, handle: Handle, point: DVec3) -> CmdResult {
        if handle.is_null() {
            return self.on_point(point);
        }
        let Some(entity) = self.picked_entity.take() else {
            return CmdResult::NeedPoint;
        };
        match self.step {
            Step::ObjectOrTwoPoints => {
                let Some(reference) = Self::picked_reference(&entity, handle, point) else {
                    return Self::invalid_object();
                };
                CmdResult::AddHorizontalConstraint {
                    selection: HorizontalConstraintSelection::Reference(reference),
                    direction: self.direction(),
                    label: "Horizontal constraint",
                }
            }
            Step::FirstPoint | Step::SecondPoint(_) => {
                let world = acadrust::types::Vector3::new(point.x, point.y, point.z);
                if !is_parametric_point_near(&entity, world) {
                    return Self::invalid_point();
                }
                let pick = Self::point_pick(Some(handle), point);
                match self.step {
                    Step::FirstPoint => {
                        self.step = Step::SecondPoint(pick);
                        CmdResult::NeedPoint
                    }
                    Step::SecondPoint(first) => self.finish_points(first, pick),
                    Step::ObjectOrTwoPoints => unreachable!(),
                }
            }
        }
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        let pick = Self::point_pick(None, point);
        match self.step {
            Step::ObjectOrTwoPoints | Step::FirstPoint => {
                self.step = Step::SecondPoint(pick);
                CmdResult::NeedPoint
            }
            Step::SecondPoint(first) => self.finish_points(first, pick),
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        if matches!(self.step, Step::ObjectOrTwoPoints) {
            self.step = Step::FirstPoint;
            CmdResult::NeedPoint
        } else {
            CmdResult::Cancel
        }
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

inventory::submit!(crate::command::CommandRegistration {
    names: &["GCHORIZONTAL"]
});
