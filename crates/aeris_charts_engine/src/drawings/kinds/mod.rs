//! B8 drawing families. Each family module owns its tool specs (wire ids inside its reserved
//! range), kind defaults, geometry (resolved into shared [`DrawingParts`]), typed options, schema
//! additions, and tests. The engine reaches a family only through the [`DrawingFamily`] hook table
//! its specs reference: a closed, compile-time table rather than a plugin registry, so adding a
//! family never edits the frame lowering, the hit tester, or another family.
//!
//! 扩展流程、wire id 范围与测试清单见 `docs/architecture/engine/drawing-families.md`。
//! 共享的单一列表注册表为每个族保留一个 `// B8: <family> — begin/end` 块，
//! 各族仅修改自己的块。

use aeris_charts_render::shape::Point;

use super::handles::{DrawingHandle, HandleDrag};
use super::parts::{DrawingParts, PartContext};
use super::Drawing;
use crate::{
    ChartEngine, DrawingKindOptions, DrawingPoint, DrawingPropertyDescriptor, DrawingToolOptions,
};

// B8: lines — begin
pub(crate) mod lines;
// B8: lines — end
// B8: channels — begin
pub(crate) mod channels;
// B8: channels — end
// B8: fibonacci — begin
pub(crate) mod fibonacci;
// B8: fibonacci — end
// B8: pitchforks_gann — begin
pub(crate) mod pitchforks_gann;
// B8: pitchforks_gann — end
// B8: projection_annotations — begin
pub(crate) mod projection_annotations;
// B8: projection_annotations — end
// B8: patterns_elliott_cycles — begin
pub(crate) mod patterns_elliott_cycles;
// B8: patterns_elliott_cycles — end
// B8: shapes — begin
pub(crate) mod shapes;
// B8: shapes — end

/// One family's engine hooks. Each family declares one `static FAMILY` (the unique address its
/// specs reference), built with [`DrawingFamily::new`] from its two required hooks and then
/// overriding optional hooks by assignment. Prefer these hooks over new ones. A new hook goes in
/// the adding family's block below and gets its default in the same block of
/// [`DrawingFamily::new`], so every other family compiles unchanged after a merge; once a second
/// family needs it, it moves out of the blocks with the shared hooks. A hook the foundation adds
/// for every family (such as `close_placement`) lives with the shared hooks from the start.
///
/// Hooks run inside frame construction and hit testing while the drawing runtime cache is
/// borrowed: they may read the engine (coordinate conversion, formatters, text measurement, data)
/// but must not call candidate queries or the cached anchor-geometry accessors.
pub(crate) struct DrawingFamily {
    /// Kind defaults applied by `Drawing::new` after the common defaults, so every creation,
    /// template, restore, paste, and schema default starts from them. Default: none.
    pub(crate) apply_defaults: fn(&mut Drawing),
    /// Resolve the body, decorations, and family labels into shared parts in caller px.
    pub(crate) build_parts: fn(&PartContext<'_>, &mut DrawingParts),
    /// Conservative CSS-px reach of decorations beyond the anchors' box, used as the screen
    /// culling pad. Must be cheap: candidate queries call it when a drawing's text key changes.
    /// Default: 0 (everything the family paints stays within the anchors' box).
    pub(crate) decoration_extent: fn(&ChartEngine, &Drawing) -> f64,
    /// Kind-specific schema descriptors appended after the common ones, whose defaults already
    /// follow the template drawing ([`apply_template_defaults`]). Default: none.
    pub(crate) extend_schema: fn(&Drawing, &mut Vec<DrawingPropertyDescriptor>),
    /// Typed kind-option projection of the live drawing.
    pub(crate) kind_options: fn(&Drawing) -> DrawingKindOptions,
    /// The family renders the common `labels` itself; the generic label pass skips its tools.
    /// Default: false.
    pub(crate) owns_labels: bool,
    /// Conservative media-px box of everything a drawing paints except text (which
    /// `decoration_extent` pads), from its anchors in media px, for geometry whose reach is
    /// screen-derived and so escapes every data-space box: tines and level lines, a circle
    /// through its rim anchor, a fixed-size square. Such a tool declares `Full` logical and price
    /// extents, so its semantic bounds never cull it, and this box replaces the whole pane as its
    /// screen culling and hit-candidate box; an extended drawing's bounds are unbounded too.
    /// `None` keeps the whole pane. Must be cheap: it runs when a drawing's coordinate key
    /// changes. Default: `None`.
    pub(crate) paint_bounds: fn(
        &ChartEngine,
        &Drawing,
        &[aeris_charts_render::shape::Point],
    ) -> Option<aeris_charts_render::shape::Rect>,
    /// The drawing's geometry reads series data (a regression's statistics, a forecast's
    /// outcome) from [`ChartEngine::drawing_source_series`], so a series data change rebuilds the
    /// drawings layer while such a drawing exists. Default: false.
    pub(crate) reads_series_data: fn(&Drawing) -> bool,
    /// `build_parts` resolves fewer anchors than the placement count, so a creation preview
    /// paints from the second anchor on (three-anchor tools show their first leg while the
    /// second anchor is being placed). Default: false (until only the last anchor remains to be
    /// placed, the preview is a guide polyline joining the placed anchors to the pointer).
    pub(crate) partial_preview: bool,
    /// Derived handles: edit the drawing's handle set in place. `handles.rs` builds it in media
    /// px from the spec's handle mode at the anchors `px`; the hook moves a handle onto derived
    /// geometry (a channel's second line, a regression's fitted line), drops one, or appends
    /// handles on derived geometry (a rotated rectangle's width handles, a pitchfork's base
    /// midpoint, a fixed square's corner) that drive [`crate::DrawingDragPart::Handle`] through
    /// `drag`. Selected-handle painting, placement previews (the placed anchors' handles),
    /// handle hit testing, keyboard handle cycling, and drag starts all read the edited set.
    /// Must be cheap. Default: the spec's set unchanged.
    pub(crate) handles: fn(&ChartEngine, &Drawing, &[Point], &mut Vec<DrawingHandle>),
    /// Complete one drag sample of any part (pointer drag or keyboard nudge, which share one
    /// session). `points` arrive as the generic drag left them: an anchor part re-anchored with
    /// snapping, magnet, and straighten applied, a body translated, and a derived handle's
    /// anchors at the drag baseline. The hook rewrites them from `drag` alone (the session
    /// recomputes from its baseline every sample; the drawing's own points and edited options
    /// are the previous sample's, so it reads only its kind and style) and returns replacement
    /// tool options, built from `drag.start_tool_options`, when the drag edits them (a fixed
    /// square's size), recorded in the same undo step. Default: the generic result, no option
    /// change.
    pub(crate) drag: fn(
        &ChartEngine,
        &Drawing,
        &HandleDrag<'_>,
        &mut [DrawingPoint],
    ) -> Option<DrawingToolOptions>,
    /// Multi-click placement close: a click within the anchor hit radius of the first placed
    /// vertex of a pending drawing with at least three vertices calls this hook to mark the
    /// drawing closed (a polyline's `closed`) and commits it without adding a vertex; while the
    /// pointer hovers there, the preview snaps onto that vertex. Enter, double-click, and
    /// Escape keep finishing open and cancelling. Default: `None` (such a click places a vertex
    /// like any other).
    pub(crate) close_placement: Option<fn(&mut Drawing)>,
    // B8: lines — begin
    // B8: lines — end
    // B8: channels — begin
    // B8: channels — end
    // B8: fibonacci — begin
    /// The drawing's complete semantic reach when its geometry extends beyond its anchors'
    /// box (level lines, time zones, projections), replacing the spec's anchor-derived culling
    /// bounds. Called on every semantic mutation, so it must be cheap. Default: `None` (the
    /// spec's `logical_extent`/`price_extent` over the anchors).
    pub(crate) bounds: fn(&Drawing) -> Option<FamilyBounds>,
    // B8: fibonacci — end
    // B8: pitchforks_gann — begin
    /// Rescale tool options measured in price units (a Gann `scale_ratio`, price per bar) by a
    /// price-basis `factor`, the segment factor of the drawing's first anchor: they belong to the
    /// data basis like the anchors' prices. Mutates only when `apply`; returns whether an option
    /// changes, or `None` when a scaled value would leave the option's valid range (which rejects
    /// the whole rescale). Default: no price-unit options (`Some(false)`).
    pub(crate) rescale_price_options:
        fn(crate::DrawingKind, &mut DrawingToolOptions, f64, bool) -> Option<bool>,
    // B8: pitchforks_gann — end
    // B8: projection_annotations — begin
    /// Engine-derived state a new drawing captures once (a bars pattern's copied bars), called
    /// with `placed == true` when the armed tool commits a placement and `false` from
    /// `add_drawing` (which paste also uses). A placement always captures, since its options come
    /// from a tool template; `add_drawing` keeps state its options already carry. Sync and restore
    /// never call it. Default: none.
    pub(crate) on_create: fn(&ChartEngine, &mut Drawing, bool),
    /// The family renders the common `text` itself; the generic text pass skips its tools.
    /// Default: false.
    pub(crate) owns_text: bool,
    /// Whether a kind's anchors are pane fractions (`logical` = x / pane width, `price` = y /
    /// pane height from the pane top) rather than time and price: the drawing stays put while
    /// the chart scrolls, carries no anchor time, ignores magnets, and never rebases or
    /// rescales. Read through [`crate::DrawingKind::pane_anchored`]. Default: no kind.
    pub(crate) pane_anchored: fn(crate::DrawingKind) -> bool,
    /// The drawing paints some parts only while focused ([`PartContext::focused`]: a note's
    /// text), so frame construction rebuilds the retained drawings layer when it becomes or
    /// stops being the hovered or selected drawing. Must be cheap: it runs per frame for the
    /// hovered and the selected drawing. Default: false.
    pub(crate) reveals_on_focus: fn(&Drawing) -> bool,
    // B8: projection_annotations — end
    // B8: patterns_elliott_cycles — begin
    // B8: patterns_elliott_cycles — end
    // B8: shapes — begin
    /// The reference box of the common box-layout `text`, in the caller px of `px` (the
    /// anchors), for a tool whose shape differs from its anchors' box (a circle placed by its
    /// center and rim). Must be cheap. Default: `None`, the anchors' box.
    pub(crate) text_box: fn(
        crate::DrawingKind,
        &[aeris_charts_render::shape::Point],
    ) -> Option<aeris_charts_render::shape::Rect>,
    // B8: shapes — end
}

impl DrawingFamily {
    /// A family with its required hooks and every optional hook at its default.
    pub(crate) const fn new(
        build_parts: fn(&PartContext<'_>, &mut DrawingParts),
        kind_options: fn(&Drawing) -> DrawingKindOptions,
    ) -> Self {
        Self {
            apply_defaults: |_| {},
            build_parts,
            decoration_extent: |_, _| 0.0,
            extend_schema: |_, _| {},
            kind_options,
            owns_labels: false,
            paint_bounds: |_, _, _| None,
            reads_series_data: |_| false,
            partial_preview: false,
            handles: |_, _, _, _| {},
            drag: |_, _, _, _| None,
            close_placement: None,
            // B8: lines — begin
            // B8: lines — end
            // B8: channels — begin
            // B8: channels — end
            // B8: fibonacci — begin
            bounds: |_| None,
            // B8: fibonacci — end
            // B8: pitchforks_gann — begin
            rescale_price_options: |_, _, _, _| Some(false),
            // B8: pitchforks_gann — end
            // B8: projection_annotations — begin
            on_create: |_, _, _| {},
            owns_text: false,
            pane_anchored: |_| false,
            reveals_on_focus: |_| false,
            // B8: projection_annotations — end
            // B8: patterns_elliott_cycles — begin
            // B8: patterns_elliott_cycles — end
            // B8: shapes — begin
            text_box: |_, _| None,
            // B8: shapes — end
        }
    }
}

/// Replace the common schema defaults with the template drawing's resolved values (a family's
/// kind defaults, such as a ray's `extend_right`), keeping every other descriptor field.
pub(crate) fn apply_template_defaults(
    template: &Drawing,
    properties: &mut [DrawingPropertyDescriptor],
) {
    let serde_json::Value::Object(options) = template.options_json() else {
        return;
    };
    for property in properties.iter_mut() {
        if let Some(value) = options.get(&property.name) {
            property.default = value.clone();
        }
    }
}

/// A family drawing's semantic reach from the [`DrawingFamily::bounds`] hook: the logical and
/// price ranges its geometry paints, each `None` when unbounded in that dimension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FamilyBounds {
    pub(crate) logical: Option<(f64, f64)>,
    pub(crate) price: Option<(f64, f64)>,
}

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;

    use super::DrawingFamily;
    use crate::drawings::DRAWING_TOOL_SPECS;
    use crate::DrawingKind;

    /// Reserved wire-id ranges; core tools carry no family hooks.
    fn reserved_ranges() -> Vec<(Option<&'static DrawingFamily>, RangeInclusive<u8>)> {
        vec![
            (None, 0..=31),
            // B8: lines — begin
            (Some(&super::lines::FAMILY), 32..=47),
            // B8: lines — end
            // B8: channels — begin (48..=63)
            (Some(&super::channels::FAMILY), 48..=63),
            // B8: channels — end
            // B8: fibonacci — begin (64..=95)
            (Some(&super::fibonacci::FAMILY), 64..=95),
            // B8: fibonacci — end
            // B8: pitchforks_gann — begin (96..=127)
            (Some(&super::pitchforks_gann::FAMILY), 96..=127),
            // B8: pitchforks_gann — end
            // B8: projection_annotations — begin (128..=159)
            (Some(&super::projection_annotations::FAMILY), 128..=159),
            // B8: projection_annotations — end
            // B8: patterns_elliott_cycles — begin (160..=191)
            (Some(&super::patterns_elliott_cycles::FAMILY), 160..=191),
            // B8: patterns_elliott_cycles — end
            // B8: shapes — begin (192..=223)
            (Some(&super::shapes::FAMILY), 192..=223),
            // B8: shapes — end
        ]
    }

    #[test]
    fn new_families_start_from_neutral_optional_hooks() {
        fn no_parts(_: &super::PartContext<'_>, _: &mut super::DrawingParts) {}
        let family = DrawingFamily::new(no_parts, |_| crate::DrawingKindOptions::Generic);
        let mut drawing = crate::Drawing::new(1, DrawingKind::TrendLine, 0, Vec::new());
        let untouched = drawing.clone();
        (family.apply_defaults)(&mut drawing);
        assert_eq!(drawing, untouched);
        let mut properties = Vec::new();
        (family.extend_schema)(&drawing, &mut properties);
        assert!(properties.is_empty());
        let chart = crate::ChartEngine::new(100.0, 100.0, 1.0);
        assert_eq!((family.decoration_extent)(&chart, &drawing), 0.0);
        assert!(!family.owns_labels);
        assert!((family.paint_bounds)(&chart, &drawing, &[(0.0, 0.0), (1.0, 1.0)]).is_none());
        assert!(!(family.reads_series_data)(&drawing));
        assert!(!family.partial_preview);
        let px = [(0.0, 0.0), (1.0, 1.0)];
        let mut handles =
            super::super::handles::handle_set(crate::drawings::DrawingHandleMode::Anchors, &px);
        let untouched = handles.clone();
        (family.handles)(&chart, &drawing, &px, &mut handles);
        assert_eq!(handles, untouched);
        let start = [crate::DrawingPoint {
            logical: 1.0,
            price: 2.0,
        }];
        let mut points = start;
        let drag = super::HandleDrag {
            part: crate::DrawingDragPart::Anchor(0),
            start_points: &start,
            start_px: &px[..1],
            start_tool_options: &drawing.tool_options,
            target: start[0],
            target_px: px[0],
            straighten: false,
            keyboard_step: None,
        };
        assert!((family.drag)(&chart, &drawing, &drag, &mut points).is_none());
        assert_eq!(points, start);
        assert!(family.close_placement.is_none());
    }

    #[test]
    fn every_tool_sits_in_its_family_range_with_unique_identity() {
        let ranges = reserved_ranges();
        for (index, spec) in DRAWING_TOOL_SPECS.iter().enumerate() {
            let (family, _) = ranges
                .iter()
                .find(|(_, range)| range.contains(&spec.wire_id))
                .unwrap_or_else(|| panic!("{} wire id {} is unreserved", spec.name, spec.wire_id));
            assert_eq!(
                family.map(|family| family as *const DrawingFamily),
                spec.family.map(|family| family as *const DrawingFamily),
                "{} is registered outside its family range",
                spec.name
            );
            assert!(
                DRAWING_TOOL_SPECS[index + 1..]
                    .iter()
                    .all(|other| other.wire_id != spec.wire_id && other.name != spec.name),
                "{} duplicates a wire id or name",
                spec.name
            );
            assert_eq!(
                spec.kind.spec().name,
                spec.name,
                "spec() dispatch for {}",
                spec.name
            );
            assert_eq!(DrawingKind::from_u8(spec.wire_id), Some(spec.kind));
            assert_eq!(DrawingKind::from_name(spec.name), Some(spec.kind));
            assert_eq!(
                serde_json::to_value(spec.kind).unwrap(),
                serde_json::json!(spec.name),
                "serde name of {} matches the wire name",
                spec.name
            );
        }
    }
}
