use acadrust::{EntityType, Handle};
use glam::DVec3;

use crate::command::{CadCommand, CmdResult};
use crate::scene::parametric_constraints::{ConstraintKind, ParametricRef};
use crate::t;

pub struct SmoothConstraintCommand {
    picked_entity: Option<EntityType>,
}

impl SmoothConstraintCommand {
    pub fn new() -> Self {
        Self {
            picked_entity: None,
        }
    }

    fn point(point: acadrust::types::Vector3) -> DVec3 {
        DVec3::new(point.x, point.y, point.z)
    }

    fn nearest_marker(points: &[acadrust::types::Vector3], point: DVec3) -> Option<i32> {
        points
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (Self::point(**left) - point)
                    .length_squared()
                    .total_cmp(&(Self::point(**right) - point).length_squared())
            })
            .map(|(index, _)| index as i32)
    }

    fn source_reference(entity: &EntityType, handle: Handle, point: DVec3) -> Option<ParametricRef> {
        let EntityType::Spline(spline) = entity else {
            return None;
        };
        if spline.flags.closed || spline.flags.periodic {
            return None;
        }
        let points = crate::scene::dimension_assoc::source_points(entity);
        let endpoints = [*points.first()?, *points.last()?];
        let marker = Self::nearest_marker(&endpoints, point)?;
        Some(ParametricRef::point(handle, marker))
    }

    fn invalid_selection() -> CmdResult {
        CmdResult::ReportError(t!("No valid constraint point found.").into_owned())
    }
}

impl CadCommand for SmoothConstraintCommand {
    fn name(&self) -> &'static str {
        "GCSMOOTH"
    }

    fn prompt(&self) -> String {
        t!("GCSMOOTH  Select first spline curve:").into_owned()
    }

    fn needs_entity_pick(&self) -> bool {
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
            return CmdResult::NeedPoint;
        }
        let Some(entity) = self.picked_entity.take() else {
            return CmdResult::NeedPoint;
        };
        let Some(source) = Self::source_reference(&entity, handle, point) else {
            return Self::invalid_selection();
        };
        CmdResult::AddParametricConstraint {
            kind: ConstraintKind::Smooth,
            // The host resolves the target from an existing Coincident
            // relation.  GCSMOOTH itself accepts only the spline selection.
            refs: vec![source],
            driving_param: None,
            label: "Smooth constraint",
        }
    }

    fn on_point(&mut self, _point: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}
