//! B8 drawing families. Exactly one renderer owns each catalog kind, chosen by `spec().family`:
//! upstream's catalog (wire ids `0..=12` and `16..=84`) carries no family and is rendered by the
//! upstream implementation (`geometry.rs` body resolver, frame arm, hit code), over which a stored
//! option may layer shared parts (the frame's `push_parts` and the hit tester's `parts_hit`, which
//! a family's parts share, with [`upstream_decoration_extent`], [`extend_upstream_schema`] and
//! [`upstream_derived_handles`] with its drag side [`drag_derived_handle`] dispatching per family);
//! the own-line tools (`240..=246`) and the three measuring ranges (`13..=15`) carry a family whose
//! module owns their tool specs, kind defaults, geometry (resolved into shared [`DrawingParts`]),
//! typed options, schema additions, and tests. The engine reaches a family only through the [`DrawingFamily`] hook table its specs
//! reference: a closed, compile-time table rather than a plugin registry, so adding a family never
//! edits the frame lowering, the hit tester, or another family.
//!
//! The modules of the retired fork families (Fibonacci, pitchforks and Gann, patterns, shapes)
//! keep their public option types and the fork's pre-merge kind defaults, which
//! [`apply_legacy_fork_defaults`] applies to documents the fork wrote, together with the fork's
//! unstored `tool_options` defaults ([`legacy_fork_tool_options`]); the Fibonacci module also
//! reads its stored options for upstream's level arms, the patterns module resolves the parts it
//! layers on upstream's polyline arm, and the pitchforks and Gann module owns what upstream's
//! pitchfork and Gann arms read from `tool_options.gann` and their derived handles, and the
//! shapes module what upstream's shape arms read from `tool_options.shape` and the caps, the
//! rotated rectangle's width handle and the curves' ends-first placement, and the
//! projection-annotations module the fork form `tool_options.projection_annotation` selects on
//! upstream's annotation arms (re-applied features). The recipe (what to add where, wire ids,
//! test checklist) lives in `docs/architecture/engine/drawing-families.md`.
//! Shared single-list registries carry one `// B8: <family> — begin/end` block per family that
//! takes part (lines, channels, fibonacci, patterns_elliott_cycles, pitchforks_gann,
//! projection_annotations, shapes).

use aeris_charts_render::shape::Point;

use super::handles::{DrawingHandle, HandleDrag};
use super::parts::{DrawingParts, PartContext};
use super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use crate::{ChartEngine, DrawingKind, DrawingKindOptions, DrawingPropertyDescriptor};

// B8: lines — begin
pub(crate) mod lines;
// B8: lines — end
// B8: channels — begin
pub(crate) mod channels;
// B8: channels — end
// B8: projection_annotations — begin
pub(crate) mod projection_annotations;
// B8: projection_annotations — end
// Retired fork families: public option types and legacy defaults only.
pub(crate) mod fibonacci;
pub(crate) mod patterns_elliott_cycles;
pub(crate) mod pitchforks_gann;
pub(crate) mod shapes;

/// One family's engine hooks. Each family declares one `static FAMILY` (the unique address its
/// specs reference), built with [`DrawingFamily::new`] from its two required hooks and then
/// overriding optional hooks by assignment. Prefer these hooks over new ones. A new hook goes in
/// the adding family's block below and gets its default in the same block of
/// [`DrawingFamily::new`], so every other family compiles unchanged after a merge; once a second
/// family needs it, it moves out of the blocks with the shared hooks. A hook the foundation adds
/// for every family lives with the shared hooks from the start.
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
    /// `build_parts` resolves fewer anchors than the placement count, so a creation preview
    /// paints from the second anchor on (three-anchor tools show their first leg while the
    /// second anchor is being placed). Default: false (until only the last anchor remains to be
    /// placed, the preview is a guide polyline joining the placed anchors to the pointer).
    pub(crate) partial_preview: bool,
    /// Derived handles: edit the drawing's handle set in place. `handles.rs` builds it in media
    /// px from the spec's handle mode at the anchors `px`; the hook moves a handle onto derived
    /// geometry (a price channel's second line) or drops one. Selected-handle painting, placement previews (the placed anchors' handles),
    /// handle hit testing, keyboard handle cycling, and drag starts all read the edited set.
    /// Must be cheap. Default: the spec's set unchanged.
    pub(crate) handles: fn(&ChartEngine, &Drawing, &[Point], &mut Vec<DrawingHandle>),
    // B8: lines — begin
    // B8: lines — end
    // B8: channels — begin
    // B8: channels — end
    // B8: projection_annotations — begin
    /// The family renders the common `text` itself; the generic text pass skips its tools.
    /// Default: false.
    pub(crate) owns_text: bool,
    // B8: projection_annotations — end
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
            partial_preview: false,
            handles: |_, _, _, _| {},
            // B8: lines — begin
            // B8: lines — end
            // B8: channels — begin
            // B8: channels — end
            // B8: projection_annotations — begin
            owns_text: false,
            // B8: projection_annotations — end
        }
    }
}

/// The CSS-px reach beyond its anchors' box of the parts an upstream-rendered kind (no family)
/// layers on its upstream arm by a stored option (`push_parts` in the frame, `parts_hit` in hit
/// testing): the culling pad its family would declare in `decoration_extent`. Must be cheap;
/// cached like a family's. 0 for a kind that layers none.
pub(crate) fn upstream_decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let mut extent: f64 = 0.0;
    // B8: lines — begin
    extent = extent.max(lines::upstream_decoration_extent(engine, drawing));
    // B8: lines — end
    // B8: channels — begin
    extent = extent.max(channels::upstream_decoration_extent(engine, drawing));
    // B8: channels — end
    // B8: fibonacci — begin
    extent = extent.max(fibonacci::upstream_decoration_extent(engine, drawing));
    // B8: fibonacci — end
    // B8: patterns_elliott_cycles — begin
    extent = extent.max(patterns_elliott_cycles::upstream_decoration_extent(
        engine, drawing,
    ));
    // B8: patterns_elliott_cycles — end
    // B8: pitchforks_gann — begin
    extent = extent.max(pitchforks_gann::upstream_decoration_extent(engine, drawing));
    // B8: pitchforks_gann — end
    // B8: shapes — begin
    extent = extent.max(shapes::upstream_decoration_extent(engine, drawing));
    // B8: shapes — end
    // B8: projection_annotations — begin
    extent = extent.max(projection_annotations::upstream_decoration_extent(
        engine, drawing,
    ));
    // B8: projection_annotations — end
    extent
}

/// Append the `tool_options.*` descriptors an upstream-rendered kind (no family) reads through
/// its layered parts, after the common ones (the family path's `extend_schema`; `template` is the
/// kind's `Drawing::new`).
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    template: &Drawing,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    // B8: lines — begin
    lines::extend_upstream_schema(kind, template, properties);
    // B8: lines — end
    // B8: channels — begin
    channels::extend_upstream_schema(kind, template, properties);
    // B8: channels — end
    // B8: fibonacci — begin
    fibonacci::extend_upstream_schema(kind, template, properties);
    // B8: fibonacci — end
    // B8: patterns_elliott_cycles — begin
    patterns_elliott_cycles::extend_upstream_schema(kind, template, properties);
    // B8: patterns_elliott_cycles — end
    // B8: pitchforks_gann — begin
    pitchforks_gann::extend_upstream_schema(kind, properties);
    // B8: pitchforks_gann — end
    // B8: shapes — begin
    shapes::extend_upstream_schema(kind, properties);
    // B8: shapes — end
}

/// Edit the handles of an upstream-rendered kind (no family) for derived geometry, the upstream
/// side of a family's `handles` hook: first upstream's projection moves the handles of anchors
/// that only parameterize a shape onto the stroke they control ([`project_anchor_handles`]),
/// then the fork replaces one (a fixed Gann square's corner, a coincident signpost's pole top) or
/// appends one (a pitchfork's base midpoint, a rotated rectangle's width). `handles.rs` builds the
/// spec's set at the media-px anchors `px` and every reader of the set (painting, previews, hit
/// testing, keyboard cycling, drag starts) sees the edited set; [`drag_derived_handle`] resolves a
/// derived `Handle`'s drags. Must be cheap.
pub(crate) fn upstream_derived_handles(
    engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    handles: &mut Vec<DrawingHandle>,
) {
    project_anchor_handles(engine, drawing, px, handles);
    // B8: pitchforks_gann — begin
    pitchforks_gann::derived_handles(engine, drawing, px, handles);
    // B8: pitchforks_gann — end
    // B8: shapes — begin
    shapes::derived_handles(drawing, px, handles);
    // B8: shapes — end
    // B8: projection_annotations — begin
    projection_annotations::derived_handles(drawing, px, handles);
    // B8: projection_annotations — end
}

/// Upstream's on-stroke handles (`geometry::anchor_handle_points`) of an upstream-rendered kind
/// whose anchors at media px `px` partly only parameterize its shape: a regression's on its fitted
/// line's ends, a channel's and a Fibonacci channel's width control on its second line, a rotated
/// rectangle's depth control on its far side. Only those kinds resolve their geometry here, so
/// the other kinds' handle sets cost what they did. The anchor drag moves each anchor by the
/// pointer delta, so its handle follows its stroke.
fn project_anchor_handles(
    engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    handles: &mut [DrawingHandle],
) {
    use DrawingKind::*;
    if !matches!(
        drawing.kind,
        RegressionTrend
            | ParallelChannel
            | FlatTopChannel
            | FlatBottomChannel
            | FibonacciChannel
            | RotatedRectangle
    ) {
        return;
    }
    let Some(pane) = engine.panes.get(drawing.pane_index) else {
        return;
    };
    // A regression's fitted points follow its anchors in its render px.
    let render;
    let px = if drawing.kind == RegressionTrend {
        let Some(full) = engine.drawing_render_px(drawing) else {
            return;
        };
        render = full;
        &render[..]
    } else {
        px
    };
    let Some(geometry) = super::resolve_drawing_geometry(
        drawing.kind,
        px,
        engine.pane_w,
        pane.top,
        pane.height,
        super::DrawingGeometryOptions::for_drawing(drawing, 1.0),
    ) else {
        return;
    };
    let points =
        super::anchor_handle_points(drawing.kind, px, drawing.points.len(), &geometry.body);
    for handle in handles {
        if let crate::DrawingDragPart::Anchor(index) = handle.part
            && let Some(&point) = points.get(index)
        {
            handle.point = point;
        }
    }
}

/// Resolve one drag sample of a derived `Handle` part of an upstream-rendered kind (the drag side
/// of [`upstream_derived_handles`]): `points` holds the baseline anchors and receives the dragged
/// ones; the returned tool options (an edit the drag makes, such as a fixed square's scale ratio)
/// replace the drawing's, and the drag's history entry and cancellation restore them with the
/// anchors. `None` rejects the sample: the drag discards `points` and keeps its last valid
/// sample, as an anchor drag does past the data.
pub(crate) fn drag_derived_handle(
    engine: &ChartEngine,
    drawing: &Drawing,
    sample: &HandleDrag<'_>,
    points: &mut [crate::DrawingPoint],
) -> Option<Option<crate::DrawingToolOptions>> {
    match drawing.kind {
        // B8: shapes — begin
        DrawingKind::RotatedRectangle => shapes::drag_handle(engine, drawing, sample, points),
        // B8: shapes — end
        // B8: projection_annotations — begin
        DrawingKind::Signpost => projection_annotations::drag_handle(sample, points),
        // B8: projection_annotations — end
        // B8: pitchforks_gann — begin
        _ => pitchforks_gann::drag_handle(engine, drawing, sample, points),
        // B8: pitchforks_gann — end
    }
}

/// After one anchor drag sample (pointer or keyboard) moved `points[index]` of an
/// upstream-rendered kind, re-derive the anchors that keep its derived geometry on screen (the
/// anchor side of [`upstream_derived_handles`]): `start_points` are the drag baseline's anchors
/// and `start_px` their media px. A rotated rectangle keeps its width while its edge turns, a
/// signpost coincident at the baseline keeps its top on its foot.
pub(crate) fn follow_anchor_drag(
    engine: &ChartEngine,
    drawing: &Drawing,
    index: usize,
    start_points: &[crate::DrawingPoint],
    start_px: &[Point],
    points: &mut [crate::DrawingPoint],
) {
    // B8: shapes — begin
    shapes::follow_anchor_drag(engine, drawing, index, start_px, points);
    // B8: shapes — end
    // B8: projection_annotations — begin
    projection_annotations::follow_anchor_drag(drawing, index, start_points, points);
    // B8: projection_annotations — end
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

/// Reset a drawing of the upstream catalog's B8 tools (wire ids `16..=84`, the tools the fork
/// rendered before the upstream sync) to the style the fork's pre-merge `Drawing::new` gave the
/// same tool: the fork's common defaults (no caps, extensions, fill, labels, levels, text, or box
/// colors; the default color, the fork tool's width, a solid line, and the fork's text alignment),
/// then the fork family's kind defaults. Documents the fork wrote omitted every style value equal
/// to those defaults, so restore applies this between `Drawing::new` and the document's own style
/// (see `persistence.rs`). Flat fields the fork did not have (`gann_fans`, `level_*`,
/// `wave_degree`, icons, bars patterns, regression deviations) keep upstream's defaults; the fork's
/// `tool_options` defaults reach them through `drawing_contract::take_legacy_flat_options`, and
/// its option blocks through [`merge_legacy_fork_tool_options`]. A no-op for core tools, the
/// own-line tools, and the ranges: their documents always stored their width (which upstream's
/// 1 px default changed), and their other defaults did not change.
pub(crate) fn apply_legacy_fork_defaults(drawing: &mut Drawing) {
    let kind = drawing.kind;
    if !matches!(kind.spec().wire_id, 16..=84) {
        return;
    }
    drawing.stroke_start = Default::default();
    drawing.stroke_end = Default::default();
    drawing.extend_left = false;
    drawing.extend_right = false;
    drawing.fill_enabled = false;
    drawing.labels.clear();
    drawing.levels.clear();
    drawing.color = crate::DRAWING_DEFAULT_COLOR.to_string();
    drawing.width = legacy_fork_width(kind);
    drawing.style = aeris_charts_render::draw_list::LineStyle::Solid;
    drawing.text.clear();
    drawing.text_size = None;
    (drawing.text_h_align, drawing.text_v_align) = if legacy_fork_segment_label(kind) {
        (DrawingTextHAlign::Right, DrawingTextVAlign::Top)
    } else {
        (DrawingTextHAlign::Center, DrawingTextVAlign::Middle)
    };
    drawing.box_color = None;
    drawing.box_border_color = None;
    lines::legacy_defaults(drawing);
    channels::legacy_defaults(drawing);
    fibonacci::legacy_defaults(drawing);
    pitchforks_gann::legacy_defaults(drawing);
    projection_annotations::legacy_defaults(drawing);
    patterns_elliott_cycles::legacy_defaults(drawing);
    shapes::legacy_defaults(drawing);
}

/// The fork's `tool_options` default of a tool of the upstream catalog's B8 range, as the block
/// name and the keys it fills in, where the fork's default differs from the upstream-neutral
/// default of the option type: the line tools' stats box (`line`), the channels' middle line and
/// Pearson's R (`channel`), the Fibonacci trend line, fan grid, and vertical label placement
/// (`fibonacci`), the Gann box's time levels and the squares' stats box (`gann`), and the
/// fork-form marker of the annotations (`projection_annotation`). The fork skipped option values
/// that were unset or at its defaults when writing, so its documents carry none of these;
/// [`merge_legacy_fork_tool_options`] puts them under what such a document stored. `None` for a
/// tool without one (the triangle pattern's apex sides are flat fields and come with
/// [`apply_legacy_fork_defaults`]).
pub(crate) fn legacy_fork_tool_options(
    kind: DrawingKind,
) -> Option<(&'static str, serde_json::Value)> {
    lines::legacy_tool_options(kind)
        .or_else(|| channels::legacy_tool_options(kind))
        .or_else(|| fibonacci::legacy_tool_options(kind))
        .or_else(|| pitchforks_gann::legacy_tool_options(kind))
        .or_else(|| projection_annotations::legacy_tool_options(kind))
}

/// Merge [`legacy_fork_tool_options`] of `kind` under `tool_options` (a `tool_options` object the
/// fork wrote, or what is left of one after `drawing_contract::take_legacy_flat_options`): a
/// missing block is added, and a stored block keeps every key it has and gains the missing ones.
/// A stored `null` or malformed block is left for the caller's validation.
pub(crate) fn merge_legacy_fork_tool_options(
    kind: DrawingKind,
    tool_options: &mut serde_json::Value,
) {
    let (Some((name, serde_json::Value::Object(defaults))), Some(stored)) =
        (legacy_fork_tool_options(kind), tool_options.as_object_mut())
    else {
        return;
    };
    let block = stored
        .entry(name)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(block) = block.as_object_mut() {
        for (key, value) in defaults {
            block.entry(key).or_insert(value);
        }
    }
}

/// The fork tool's default stroke width (its family spec's `default_width`).
fn legacy_fork_width(kind: DrawingKind) -> f64 {
    use DrawingKind::*;
    match kind {
        Ray | ExtendedLine | InfoLine | TrendAngle | CrossLine | ArrowLine => 2.0,
        ParallelChannel | FlatTopChannel | FlatBottomChannel | DisjointChannel => 2.0,
        Forecast => 2.0,
        PatternXabcd
        | PatternCypher
        | PatternAbcd
        | PatternHeadShoulders
        | PatternTriangle
        | PatternThreeDrives
        | ElliottImpulse
        | ElliottCorrection
        | ElliottTriangle
        | ElliottDoubleCombination
        | ElliottTripleCombination
        | TimeCycles
        | SineLine => 2.0,
        RotatedRectangle | Ellipse | Circle | Triangle | Arc | Curve | DoubleCurve | Polyline => {
            2.0
        }
        Highlighter => 20.0,
        _ => 1.0,
    }
}

/// Whether the fork tool's text label ran along its segment (its spec's `Segment` text layout),
/// which the fork's `Drawing::new` aligned top-right.
fn legacy_fork_segment_label(kind: DrawingKind) -> bool {
    use DrawingKind::*;
    matches!(
        kind,
        Ray | ExtendedLine
            | InfoLine
            | TrendAngle
            | ArrowLine
            | ParallelChannel
            | FlatTopChannel
            | FlatBottomChannel
            | DisjointChannel
    )
}

#[cfg(test)]
mod tests {
    use super::DrawingFamily;
    use crate::DrawingKind;
    use crate::drawings::DRAWING_TOOL_SPECS;

    /// The family (if any) owning each wire id: upstream's catalog has none, the ranges and the
    /// own-line tools have theirs.
    fn expected_family(wire_id: u8) -> Option<&'static DrawingFamily> {
        match wire_id {
            0..=12 | 16..=84 => None,
            13..=15 | 245..=246 => Some(&super::projection_annotations::FAMILY),
            240..=243 => Some(&super::lines::FAMILY),
            244 => Some(&super::channels::FAMILY),
            other => panic!("wire id {other} is outside the catalog"),
        }
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
        assert!(!family.owns_text);
        assert!(!family.partial_preview);
        let px = [(0.0, 0.0), (1.0, 1.0)];
        let mut handles =
            super::super::handles::handle_set(crate::drawings::DrawingHandleMode::Anchors, &px);
        let untouched = handles.clone();
        (family.handles)(&chart, &drawing, &px, &mut handles);
        assert_eq!(handles, untouched);
    }

    #[test]
    fn every_tool_has_its_owner_and_a_unique_identity() {
        let mut upstream = Vec::new();
        for (index, spec) in DRAWING_TOOL_SPECS.iter().enumerate() {
            assert_eq!(
                expected_family(spec.wire_id).map(|family| family as *const DrawingFamily),
                spec.family.map(|family| family as *const DrawingFamily),
                "{} (wire id {}) has the wrong renderer owner",
                spec.name,
                spec.wire_id
            );
            if spec.wire_id <= 84 {
                upstream.push(spec.wire_id);
            }
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
            assert_eq!(
                serde_json::from_value::<DrawingKind>(serde_json::json!(spec.name)).unwrap(),
                spec.kind
            );
        }
        // Upstream's catalog is contiguous (listed in upstream's order, which places the polyline
        // and highlighter before the rotated rectangle); the own-line block is complete.
        upstream.sort_unstable();
        assert_eq!(upstream, (0..=84).collect::<Vec<u8>>());
        let own_line = DRAWING_TOOL_SPECS
            .iter()
            .filter(|spec| spec.wire_id >= 240)
            .map(|spec| spec.wire_id)
            .collect::<Vec<_>>();
        assert_eq!(own_line, (240..=246).collect::<Vec<u8>>());
        assert_eq!(DRAWING_TOOL_SPECS.len(), 92);
        // Legacy names resolve but are never catalog names.
        for (name, kind) in crate::drawings::LEGACY_DRAWING_KIND_NAMES {
            assert_eq!(DrawingKind::from_name(name), Some(kind), "{name}");
            assert!(DRAWING_TOOL_SPECS.iter().all(|spec| spec.name != name));
        }
    }

    #[test]
    fn legacy_fork_defaults_touch_only_the_tools_the_fork_rendered() {
        for spec in DRAWING_TOOL_SPECS {
            let fresh = crate::Drawing::new(1, spec.kind, 0, Vec::new());
            let mut legacy = fresh.clone();
            super::apply_legacy_fork_defaults(&mut legacy);
            if !(16..=84).contains(&spec.wire_id) {
                assert_eq!(legacy, fresh, "{} keeps its defaults", spec.name);
            }
            // Idempotent, and never touching identity, placement, or the flat fields the fork
            // did not have.
            let mut twice = legacy.clone();
            super::apply_legacy_fork_defaults(&mut twice);
            assert_eq!(twice, legacy, "{}", spec.name);
            assert_eq!(legacy.kind, fresh.kind);
            assert_eq!(legacy.points, fresh.points);
            assert_eq!(legacy.gann_fans, fresh.gann_fans);
            assert_eq!(legacy.wave_degree, fresh.wave_degree);
            assert_eq!(legacy.icon_size, fresh.icon_size);
            assert_eq!(legacy.regression_deviations, fresh.regression_deviations);
            // The fork's unstored option defaults: only for the tools it rendered, each a valid
            // block that differs from the option type's (upstream-neutral) defaults, merged
            // under a document's own keys idempotently.
            let Some((name, block)) = super::legacy_fork_tool_options(spec.kind) else {
                continue;
            };
            assert!((16..=84).contains(&spec.wire_id), "{}", spec.name);
            let mut merged = serde_json::json!({});
            super::merge_legacy_fork_tool_options(spec.kind, &mut merged);
            assert_eq!(merged, serde_json::json!({ name: block }), "{}", spec.name);
            let mut twice = merged.clone();
            super::merge_legacy_fork_tool_options(spec.kind, &mut twice);
            assert_eq!(twice, merged, "{}", spec.name);
            let options = serde_json::from_value::<crate::DrawingToolOptions>(merged).unwrap();
            assert!(options.validate(), "{}", spec.name);
            assert_ne!(
                options,
                crate::DrawingToolOptions::default(),
                "{}",
                spec.name
            );
            // A block the document stored keeps its keys; a reset stays a reset.
            let mut reset = serde_json::json!({ name: null });
            super::merge_legacy_fork_tool_options(spec.kind, &mut reset);
            assert_eq!(reset, serde_json::json!({ name: null }), "{}", spec.name);
        }
    }
}
