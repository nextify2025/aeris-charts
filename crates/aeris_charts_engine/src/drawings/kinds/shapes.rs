//! B8 Shapes options and legacy defaults. Upstream renders the rotated rectangle, ellipse, circle,
//! triangle, arc, curve, double curve, polyline, and highlighter from its catalog spec; this module
//! keeps the fork's public option block ([`ShapeToolOptions`]) and, for documents the fork wrote,
//! its pre-merge kind defaults ([`legacy_defaults`]).

use super::super::Drawing;
use crate::DrawingKind;

/// The fork's shapes options (`tool_options.shape`); absent fields keep their defaults. Stored and
/// persisted, but not rendered: upstream's polyline is always open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShapeToolOptions {
    /// Polyline only: the fork joined the last vertex back to the first and filled the enclosed
    /// region. The other shapes ignore it.
    pub closed: bool,
}

/// The fork's highlighter opacity over the canonical market-warning amber.
const HIGHLIGHTER_OPACITY: f64 = 0.4;

/// The fork's pre-merge defaults of the shape tools (see [`super::apply_legacy_fork_defaults`]):
/// closed shapes filled, and the highlighter in translucent amber.
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    match drawing.kind {
        DrawingKind::RotatedRectangle
        | DrawingKind::Ellipse
        | DrawingKind::Circle
        | DrawingKind::Triangle
        | DrawingKind::Arc
        | DrawingKind::Polyline => drawing.fill_enabled = true,
        DrawingKind::Highlighter => {
            let (r, g, b) = aeris_charts_core::style::MARKET_WARNING_RGB;
            drawing.color = format!("rgba({r}, {g}, {b}, {HIGHLIGHTER_OPACITY})");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
