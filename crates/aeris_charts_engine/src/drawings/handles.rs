//! The one editable-handle set of a drawing. Selected-handle painting, handle hit testing, and
//! keyboard handle cycling all iterate this set, so a handle can never be painted without being
//! draggable, or reachable by keyboard without being painted. A family's `handles` hook edits
//! the set (derived handles), and its `drag` hook completes every drag sample through
//! [`HandleDrag`], so pointer drags, keyboard nudges, and magnet snapping share one path.

use super::{ChartEngine, Drawing, DrawingDragPart, DrawingHandleMode, DrawingPoint};

/// Painted form of a handle (sizes are the frame's shared anchor radius and border).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandleShape {
    /// Border disc under a fill disc (anchors, rectangle corners, a position's entry).
    Disc,
    /// Square border under a square fill (a position's target, width, and stop controls).
    Square,
    /// Slightly rounded square (a rectangle's edge midpoints).
    RoundedSquare,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawingHandle {
    /// Caller-px position (media px for hit testing, bitmap px for painting).
    pub(crate) point: (f64, f64),
    /// The drag part the handle drives; the index is the keyboard handle order.
    pub(crate) part: DrawingDragPart,
    pub(crate) cursor: &'static str,
    pub(crate) shape: HandleShape,
}

/// The handles of a drawing with `mode` whose anchors sit at caller-px `px`, in keyboard order:
/// every anchor, a freehand stroke's two ends, a rectangle's eight bounds handles (clockwise from
/// top-left), or a Long/Short Position's target, entry, width, and stop controls.
pub(crate) fn handle_set(mode: DrawingHandleMode, px: &[(f64, f64)]) -> Vec<DrawingHandle> {
    let handle = |index: usize, point, cursor, shape| DrawingHandle {
        point,
        part: DrawingDragPart::Anchor(index),
        cursor,
        shape,
    };
    match mode {
        DrawingHandleMode::None => Vec::new(),
        DrawingHandleMode::Endpoints if px.len() >= 2 => {
            let last = px.len() - 1;
            vec![
                handle(0, px[0], "pointer", HandleShape::Disc),
                handle(last, px[last], "pointer", HandleShape::Disc),
            ]
        }
        DrawingHandleMode::RectangleBounds if px.len() == 2 => ChartEngine::rectangle_anchors(px)
            .into_iter()
            .enumerate()
            .map(|(index, point)| {
                let shape = if index % 2 == 0 {
                    HandleShape::Disc
                } else {
                    HandleShape::RoundedSquare
                };
                handle(
                    index,
                    point,
                    ChartEngine::rectangle_anchor_cursor(index),
                    shape,
                )
            })
            .collect(),
        DrawingHandleMode::Position if px.len() == 3 => {
            let (entry, target, stop) = (px[0], px[1], px[2]);
            vec![
                handle(0, (entry.0, target.1), "ns-resize", HandleShape::Square),
                handle(1, entry, "move", HandleShape::Disc),
                handle(2, (target.0, entry.1), "ew-resize", HandleShape::Square),
                handle(3, (entry.0, stop.1), "ns-resize", HandleShape::Square),
            ]
        }
        _ => px
            .iter()
            .enumerate()
            .map(|(index, &point)| handle(index, point, "pointer", HandleShape::Disc))
            .collect(),
    }
}

/// One drag sample of a drawing, handed to its family's `drag` hook
/// ([`super::kinds::DrawingFamily::drag`]). Pointer drags and keyboard nudges build the same
/// sample; nudges never magnet-snap and carry their step in `keyboard_step`.
pub(crate) struct HandleDrag<'a> {
    /// The dragged part.
    pub(crate) part: DrawingDragPart,
    /// The anchors at the drag baseline (the press, or the last data-driven rebaseline) and
    /// their media px.
    pub(crate) start_points: &'a [DrawingPoint],
    pub(crate) start_px: &'a [(f64, f64)],
    /// The drawing's tool options at the press, the base of any option the hook edits (the live
    /// options already carry the previous sample's edit).
    pub(crate) start_tool_options: &'a crate::DrawingToolOptions,
    /// Where the dragged point goes: the handle's baseline media px moved by the pointer delta
    /// (constrained to the spec's movement axis), time-snapped and magnet-snapped like an anchor,
    /// as an anchor point and back in media px. For an anchor part it is the generic drag's
    /// result; for a body drag, the translated press point.
    pub(crate) target: DrawingPoint,
    pub(crate) target_px: (f64, f64),
    /// Shift (straighten) is held.
    pub(crate) straighten: bool,
    /// A keyboard nudge's media-px step (`None` for a pointer drag). A hook that quantizes the
    /// target (a fixed square's whole bars) steps at least one unit the way the key moved, so
    /// repeated sub-unit nudges still edit the drawing.
    pub(crate) keyboard_step: Option<(f64, f64)>,
}

impl ChartEngine {
    /// The editable handles of `drawing` whose anchors sit at media px `px`: the spec's handle
    /// mode, edited by the family's `handles` hook (derived handles). Callers scale the points
    /// for painting; every mode's handle placement is affine in the anchors.
    pub(crate) fn drawing_handle_set(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
    ) -> Vec<DrawingHandle> {
        let mut handles = handle_set(drawing.kind.spec().handles, px);
        if let Some(family) = drawing.kind.spec().family {
            (family.handles)(self, drawing, px, &mut handles);
        }
        handles
    }

    /// The anchor of `drawing` under media px `point`, time-snapped to a bar when the drawing
    /// snaps its time to data: a family drag moving anchors with a derived handle converts
    /// them like the generic drag does.
    pub(crate) fn drawing_anchor_at(
        &self,
        drawing: &Drawing,
        (x, y): (f64, f64),
    ) -> Option<DrawingPoint> {
        let point = self.drawing_anchor_from_px(
            drawing.kind,
            drawing.pane_index,
            drawing.price_scale,
            x,
            y,
        )?;
        if drawing.snap_time_to_data && !drawing.kind.pane_anchored() {
            self.snap_drawing_time_to_data(point)
        } else {
            Some(point)
        }
    }

    /// The media px of the handle of `drawing` that drives `part`, when it has one.
    pub(crate) fn drawing_handle_px(
        &self,
        drawing: &Drawing,
        part: DrawingDragPart,
    ) -> Option<(f64, f64)> {
        let px = self.drawing_px(drawing)?;
        self.drawing_handle_set(drawing, &px)
            .into_iter()
            .find(|handle| handle.part == part)
            .map(|handle| handle.point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_sets_follow_each_mode() {
        let two = [(0.0, 0.0), (10.0, 20.0)];
        assert!(handle_set(DrawingHandleMode::None, &two).is_empty());
        let anchors = handle_set(DrawingHandleMode::Anchors, &two);
        assert_eq!(anchors.len(), 2);
        assert!(anchors
            .iter()
            .all(|handle| handle.shape == HandleShape::Disc && handle.cursor == "pointer"));
        let path = [(0.0, 0.0), (5.0, 5.0), (9.0, 1.0)];
        let ends = handle_set(DrawingHandleMode::Endpoints, &path);
        assert_eq!(
            ends.iter().map(|handle| handle.part).collect::<Vec<_>>(),
            [DrawingDragPart::Anchor(0), DrawingDragPart::Anchor(2)]
        );
        let bounds = handle_set(DrawingHandleMode::RectangleBounds, &two);
        assert_eq!(bounds.len(), 8);
        assert_eq!(bounds[1].shape, HandleShape::RoundedSquare);
        assert_eq!(bounds[0].cursor, "nwse-resize");
        let position = handle_set(
            DrawingHandleMode::Position,
            &[(0.0, 50.0), (40.0, 10.0), (0.0, 70.0)],
        );
        assert_eq!(
            position
                .iter()
                .map(|handle| handle.point)
                .collect::<Vec<_>>(),
            [(0.0, 10.0), (0.0, 50.0), (40.0, 50.0), (0.0, 70.0)]
        );
        assert_eq!(position[1].cursor, "move");
    }
}
