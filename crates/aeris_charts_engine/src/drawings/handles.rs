//! The one editable-handle set of a drawing. Selected-handle painting, creation previews, handle
//! hit testing, and keyboard handle cycling all iterate this set, so a handle can never be painted
//! without being draggable, or reachable by keyboard without being painted. A family's `handles`
//! hook edits the set (derived handles), as `kinds::upstream_derived_handles` does for
//! upstream-rendered kinds: upstream's projection first puts an anchor that only parameterizes a
//! shape on the stroke it controls (`geometry::anchor_handle_points`), then the fork's derived
//! handles are added. Pointer drags, keyboard nudges, and magnet snapping share the engine's one
//! drag path, which moves every anchor bar by bar from its own slot. A derived
//! `DrawingDragPart::Handle` drags through that path too: `drawing_drag_apply` moves the handle
//! like an anchor (movement axis, time snap, magnet, bar slots) into one [`HandleDrag`] sample,
//! and `kinds::drag_derived_handle` resolves what the handle drives (anchors, and tool options
//! the drag's one history entry and cancellation restore with them).

use super::{ChartEngine, Drawing, DrawingDragPart, DrawingHandleMode, DrawingPoint, kinds};

/// A drawing handle's fill radius and border width in CSS px: the fill radius plus the border
/// makes the frame's 12 px handle (`push_handle`), which point labels clear.
pub(crate) const ANCHOR_RADIUS: f64 = 4.5;
pub(crate) const ANCHOR_BORDER_WIDTH: f64 = 1.5;

/// Painted form of a handle: the frame's one handle look (`push_handle`), round or square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandleShape {
    /// Anchors, rectangle corners, icon box corners, and a position's entry.
    Round,
    /// Slightly rounded square: a rectangle's edge midpoints, a position's target, width, and
    /// stop controls, and a signpost's post top.
    Square,
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
/// top-left), a Long/Short Position's target, entry, width, and stop controls, or one anchor's
/// handle. An icon box's corners need the resolved icon geometry, so
/// [`ChartEngine::drawing_handle_set`] builds them.
pub(crate) fn handle_set(mode: DrawingHandleMode, px: &[(f64, f64)]) -> Vec<DrawingHandle> {
    let handle = |index: usize, point, cursor, shape| DrawingHandle {
        point,
        part: DrawingDragPart::Anchor(index),
        cursor,
        shape,
    };
    match mode {
        DrawingHandleMode::None | DrawingHandleMode::IconBox => Vec::new(),
        DrawingHandleMode::Endpoints if px.len() >= 2 => {
            let last = px.len() - 1;
            vec![
                handle(0, px[0], "pointer", HandleShape::Round),
                handle(last, px[last], "pointer", HandleShape::Round),
            ]
        }
        DrawingHandleMode::OneAnchor { index, square } => {
            let index = usize::from(index);
            let shape = if square {
                HandleShape::Square
            } else {
                HandleShape::Round
            };
            px.get(index)
                .map(|&point| vec![handle(index, point, "pointer", shape)])
                .unwrap_or_default()
        }
        DrawingHandleMode::RectangleBounds if px.len() == 2 => ChartEngine::rectangle_anchors(px)
            .into_iter()
            .enumerate()
            .map(|(index, point)| {
                let shape = if index % 2 == 0 {
                    HandleShape::Round
                } else {
                    HandleShape::Square
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
                handle(1, entry, "move", HandleShape::Round),
                handle(2, (target.0, entry.1), "ew-resize", HandleShape::Square),
                handle(3, (entry.0, stop.1), "ns-resize", HandleShape::Square),
            ]
        }
        _ => px
            .iter()
            .enumerate()
            .map(|(index, &point)| handle(index, point, "pointer", HandleShape::Round))
            .collect(),
    }
}

/// One derived-handle drag sample, handed to `kinds::drag_derived_handle`. Pointer drags and
/// keyboard nudges build the same sample; nudges never magnet-snap and carry their step in
/// `keyboard_step`.
pub(crate) struct HandleDrag<'a> {
    /// The dragged part.
    pub(crate) part: DrawingDragPart,
    /// The anchors at the drag baseline (the press, or the last data-driven rebaseline) and
    /// their media px.
    pub(crate) start_points: &'a [DrawingPoint],
    pub(crate) start_px: &'a [(f64, f64)],
    /// The drawing's tool options at the press, the base of any option the drag edits (the live
    /// options already carry the previous sample's edit).
    pub(crate) start_tool_options: &'a crate::DrawingToolOptions,
    /// The dragged handle's media px at the baseline.
    pub(crate) handle_px: (f64, f64),
    /// Where the handle goes: its baseline media px moved by the pointer delta (constrained to
    /// the spec's movement axis), time-snapped and magnet-snapped like an anchor, and on the bar
    /// slot under it (a keyboard step: whole bars from the handle's own position; a rotated
    /// rectangle's width handle stays continuous), as an anchor point and back in media px.
    pub(crate) target: DrawingPoint,
    pub(crate) target_px: (f64, f64),
    /// Shift (straighten) is held.
    pub(crate) straighten: bool,
    /// A keyboard nudge's media-px step (`None` for a pointer drag). A drag that quantizes the
    /// target (a fixed square's whole bars) steps at least one unit the way the key moved, so
    /// repeated sub-unit nudges still edit the drawing.
    pub(crate) keyboard_step: Option<(f64, f64)>,
}

impl ChartEngine {
    /// The anchor of `drawing` under media px `point`, time-snapped to a bar when the drawing
    /// snaps its time to data: a derived handle that moves anchors converts them as the anchor
    /// drag does.
    pub(crate) fn drawing_anchor_at(
        &self,
        drawing: &Drawing,
        (x, y): (f64, f64),
    ) -> Option<DrawingPoint> {
        let point = self.drawing_from_px_for(drawing.pane_index, drawing.price_scale, x, y)?;
        if drawing.snap_time_to_data {
            self.snap_drawing_time_to_data(point)
        } else {
            Some(point)
        }
    }

    /// The handle mode of `drawing`: its kind's, except where the fork form keeps the fork's
    /// editing (owner policy: editing changes only where approved). A fork-form note, comment,
    /// or price note keeps no handles (its text box is its editing surface), and a fork-form
    /// signpost keeps a handle on each anchor, its pole top derived from them.
    pub(crate) fn drawing_handle_mode(&self, drawing: &Drawing) -> DrawingHandleMode {
        use super::DrawingKind;
        match drawing.kind {
            DrawingKind::Note | DrawingKind::Comment | DrawingKind::PriceNote
                if kinds::projection_annotations::fork_text_owner(drawing) =>
            {
                DrawingHandleMode::None
            }
            DrawingKind::Signpost if kinds::projection_annotations::fork_form(drawing) => {
                DrawingHandleMode::Anchors
            }
            kind => kind.spec().handles,
        }
    }

    /// The editable handles of `drawing` whose anchors sit at media px `px`: its handle mode
    /// ([`Self::drawing_handle_mode`]; an icon's four box corners, clockwise from top left,
    /// resize it about its anchor), edited by the family's `handles` hook or, for an
    /// upstream-rendered kind, [`kinds::upstream_derived_handles`] (derived handles, such as a
    /// regression trend's on its fitted line). Every caller passes media px and scales the
    /// points afterwards for painting.
    pub(crate) fn drawing_handle_set(
        &self,
        drawing: &Drawing,
        px: &[(f64, f64)],
    ) -> Vec<DrawingHandle> {
        let mode = self.drawing_handle_mode(drawing);
        let mut handles = if mode == DrawingHandleMode::IconBox {
            self.icon_box(drawing, px, 1.0)
                .map(|icon| {
                    Self::icon_box_corners(icon)
                        .into_iter()
                        .enumerate()
                        .map(|(corner, point)| DrawingHandle {
                            point,
                            part: DrawingDragPart::Anchor(corner),
                            cursor: if corner % 2 == 0 {
                                "nwse-resize"
                            } else {
                                "nesw-resize"
                            },
                            shape: HandleShape::Round,
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            handle_set(mode, px)
        };
        match drawing.kind.spec().family {
            Some(family) => (family.handles)(self, drawing, px, &mut handles),
            None => kinds::upstream_derived_handles(self, drawing, px, &mut handles),
        }
        handles
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
        assert!(
            anchors
                .iter()
                .all(|handle| handle.shape == HandleShape::Round && handle.cursor == "pointer")
        );
        let path = [(0.0, 0.0), (5.0, 5.0), (9.0, 1.0)];
        let ends = handle_set(DrawingHandleMode::Endpoints, &path);
        assert_eq!(
            ends.iter().map(|handle| handle.part).collect::<Vec<_>>(),
            [DrawingDragPart::Anchor(0), DrawingDragPart::Anchor(2)]
        );
        let bounds = handle_set(DrawingHandleMode::RectangleBounds, &two);
        assert_eq!(bounds.len(), 8);
        assert_eq!(bounds[0].shape, HandleShape::Round);
        assert_eq!(bounds[1].shape, HandleShape::Square);
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
        // One anchor's handle, square on request (the signpost's top).
        let square = handle_set(
            DrawingHandleMode::OneAnchor {
                index: 1,
                square: true,
            },
            &two,
        );
        assert_eq!(square.len(), 1);
        assert_eq!(
            (square[0].part, square[0].point, square[0].shape),
            (DrawingDragPart::Anchor(1), two[1], HandleShape::Square)
        );
        let round = handle_set(
            DrawingHandleMode::OneAnchor {
                index: 0,
                square: false,
            },
            &two,
        );
        assert_eq!(round[0].shape, HandleShape::Round);
        assert!(
            handle_set(
                DrawingHandleMode::OneAnchor {
                    index: 2,
                    square: false,
                },
                &two,
            )
            .is_empty(),
            "no handle without its anchor"
        );
        // An icon's box corners need its size, so the engine's handle set adds them.
        assert!(handle_set(DrawingHandleMode::IconBox, &two[..1]).is_empty());
    }

    /// An icon's four box corners, clockwise from top left, are round resize handles around its
    /// anchor (the stamp's center), in keyboard order, with diagonal cursors.
    #[test]
    fn an_icon_box_handle_set_is_its_four_corners() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..10).map(|i| f64::from(i) * 60.0).collect::<Vec<_>>();
        let values = vec![10.0; 10];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();
        let id = chart
            .add_drawing(
                super::super::DrawingKind::IconStamp,
                0,
                vec![super::super::DrawingPoint {
                    logical: 4.0,
                    price: 10.0,
                }],
                Some(r#"{"icon_name":"star","icon_size":40}"#),
            )
            .unwrap();
        let drawing = chart.drawing(id).unwrap();
        let center = chart.drawing_point_to_coordinate(id, 0).unwrap();
        let px = chart.drawing_px(drawing).unwrap();
        let handles = chart.drawing_handle_set(drawing, &px);
        assert_eq!(
            handles
                .iter()
                .map(|handle| (handle.part, handle.shape, handle.cursor))
                .collect::<Vec<_>>(),
            [
                (
                    DrawingDragPart::Anchor(0),
                    HandleShape::Round,
                    "nwse-resize"
                ),
                (
                    DrawingDragPart::Anchor(1),
                    HandleShape::Round,
                    "nesw-resize"
                ),
                (
                    DrawingDragPart::Anchor(2),
                    HandleShape::Round,
                    "nwse-resize"
                ),
                (
                    DrawingDragPart::Anchor(3),
                    HandleShape::Round,
                    "nesw-resize"
                ),
            ]
        );
        let corners = [(-20.0, -20.0), (20.0, -20.0), (20.0, 20.0), (-20.0, 20.0)];
        for (handle, (dx, dy)) in handles.iter().zip(corners) {
            let expected = (center.0 + dx, center.1 + dy);
            assert!(
                (handle.point.0 - expected.0).abs() < 1e-6
                    && (handle.point.1 - expected.1).abs() < 1e-6,
                "{:?} vs {expected:?}",
                handle.point
            );
        }
        assert_eq!(chart.drawing_handle_count(id), Some(4));
    }
}
