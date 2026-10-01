//! Backend-neutral resolved drawing geometry.
//!
//! A tool's semantic anchors are converted once into this small geometry vocabulary.  Frame
//! lowering and precise hit-testing both consume the same result, preventing the rendered shape
//! and the interactive shape from drifting as more drawing kinds are added.

use aeris_charts_render::draw_list::LineType;

use super::{path_arrow_points, DrawingKind, TextBox};

#[derive(Clone, Copy, Debug)]
pub(crate) enum DrawingBodyGeometry<'a> {
    Empty,
    Segment {
        a: (f64, f64),
        b: (f64, f64),
    },
    Horizontal {
        y: f64,
        x0: f64,
        x1: f64,
    },
    Vertical {
        x: f64,
        y0: f64,
        y1: f64,
    },
    Rectangle {
        left: f64,
        right: f64,
        top: f64,
        bottom: f64,
    },
    Position(PositionGeometry),
    Polyline {
        points: &'a [(f64, f64)],
        line_type: LineType,
        terminal: Option<[(f64, f64); 3]>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositionGeometry {
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) entry_y: f64,
    pub(crate) target_y: f64,
    pub(crate) stop_y: f64,
}

/// Which dimensions a measuring tool reports. Price arrows run vertically, time arrows
/// horizontally; the combined tool draws both through the box center.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeasureAxes {
    Price,
    Date,
    DatePrice,
}

impl MeasureAxes {
    pub(crate) const fn for_kind(kind: DrawingKind) -> Option<Self> {
        match kind {
            DrawingKind::PriceRange => Some(Self::Price),
            DrawingKind::DateRange => Some(Self::Date),
            DrawingKind::DateAndPriceRange => Some(Self::DatePrice),
            _ => None,
        }
    }

    pub(crate) const fn price(self) -> bool {
        matches!(self, Self::Price | Self::DatePrice)
    }

    pub(crate) const fn date(self) -> bool {
        matches!(self, Self::Date | Self::DatePrice)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositionZone {
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) y0: f64,
    pub(crate) y1: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DrawingGeometryOptions {
    pub(crate) line_width: f64,
    pub(crate) device_scale: f64,
    pub(crate) extend_left: bool,
    pub(crate) extend_right: bool,
}

impl PositionGeometry {
    fn from_points(entry: (f64, f64), target: (f64, f64), stop: (f64, f64)) -> Self {
        Self {
            left: entry.0.min(target.0),
            right: entry.0.max(target.0),
            entry_y: entry.1,
            target_y: target.1,
            stop_y: stop.1,
        }
    }

    pub(crate) fn reward_zone(self) -> PositionZone {
        PositionZone {
            left: self.left,
            right: self.right,
            y0: self.entry_y,
            y1: self.target_y,
        }
    }

    pub(crate) fn risk_zone(self) -> PositionZone {
        PositionZone {
            left: self.left,
            right: self.right,
            y0: self.entry_y,
            y1: self.stop_y,
        }
    }

    pub(crate) fn top(self) -> f64 {
        self.entry_y.min(self.target_y).min(self.stop_y)
    }

    pub(crate) fn bottom(self) -> f64 {
        self.entry_y.max(self.target_y).max(self.stop_y)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedDrawingGeometry<'a> {
    pub(crate) body: DrawingBodyGeometry<'a>,
    pub(crate) text_box: TextBox,
}

fn points_box(px: &[(f64, f64)]) -> Option<TextBox> {
    let &(first_x, first_y) = px.first()?;
    let (mut left, mut right, mut top, mut bottom) = (first_x, first_x, first_y, first_y);
    for &(x, y) in &px[1..] {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    Some(TextBox {
        left,
        right,
        top,
        bottom,
    })
}

pub(crate) fn resolve_drawing_geometry<'a>(
    kind: DrawingKind,
    px: &'a [(f64, f64)],
    pane_w: f64,
    pane_top: f64,
    pane_h: f64,
    options: DrawingGeometryOptions,
) -> Option<ResolvedDrawingGeometry<'a>> {
    if px.is_empty() {
        return None;
    }
    let body = match kind {
        DrawingKind::TrendLine => {
            let mut a = *px.first()?;
            let mut b = *px.get(1)?;
            let dx = b.0 - a.0;
            if dx.abs() > f64::EPSILON {
                let slope = (b.1 - a.1) / dx;
                if options.extend_left {
                    a.1 += (0.0 - a.0) * slope;
                    a.0 = 0.0;
                }
                if options.extend_right {
                    b.1 += (pane_w - b.0) * slope;
                    b.0 = pane_w;
                }
            }
            DrawingBodyGeometry::Segment { a, b }
        }
        DrawingKind::HorizontalLine => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: 0.0,
            x1: pane_w,
        },
        DrawingKind::HorizontalRay => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: if options.extend_left { 0.0 } else { px[0].0 },
            x1: pane_w,
        },
        DrawingKind::VerticalLine => DrawingBodyGeometry::Vertical {
            x: px[0].0,
            y0: pane_top,
            y1: pane_top + pane_h,
        },
        DrawingKind::Rectangle => {
            let (a, b) = (*px.first()?, *px.get(1)?);
            DrawingBodyGeometry::Rectangle {
                left: a.0.min(b.0),
                right: a.0.max(b.0),
                top: a.1.min(b.1),
                bottom: a.1.max(b.1),
            }
        }
        DrawingKind::Text => DrawingBodyGeometry::Empty,
        DrawingKind::Brush => DrawingBodyGeometry::Polyline {
            points: px,
            line_type: LineType::Curved,
            terminal: None,
        },
        DrawingKind::Path => DrawingBodyGeometry::Polyline {
            points: px,
            line_type: LineType::Simple,
            terminal: path_arrow_points(px, options.line_width, options.device_scale),
        },
        DrawingKind::LongPosition | DrawingKind::ShortPosition => {
            let entry = *px.first()?;
            let target = *px.get(1)?;
            let stop = *px.get(2)?;
            DrawingBodyGeometry::Position(PositionGeometry::from_points(entry, target, stop))
        }
        DrawingKind::FixedRangeVolumeProfile => DrawingBodyGeometry::Segment {
            a: *px.first()?,
            b: *px.get(1)?,
        },
        DrawingKind::AnchoredVolumeProfile | DrawingKind::AnchoredVwap => {
            DrawingBodyGeometry::Vertical {
                x: px[0].0,
                y0: pane_top,
                y1: pane_top + pane_h,
            }
        }
        // B8 family tools resolve their bodies through `kinds::` parts; only their anchors'
        // box (or the family's `text_box`) reaches this resolver, as the reference box of a
        // box-layout text label.
        _ => {
            let family = kind.spec().family;
            debug_assert!(family.is_some());
            let text_box = family
                .and_then(|family| (family.text_box)(kind, px))
                .map(|rect| TextBox {
                    left: rect.left,
                    right: rect.right,
                    top: rect.top,
                    bottom: rect.bottom,
                })
                .or_else(|| points_box(px))?;
            return Some(ResolvedDrawingGeometry {
                body: DrawingBodyGeometry::Empty,
                text_box,
            });
        }
    };

    let text_box = match body {
        DrawingBodyGeometry::Empty => {
            let (x, y) = *px.first()?;
            TextBox {
                left: x,
                right: x,
                top: y,
                bottom: y,
            }
        }
        DrawingBodyGeometry::Segment { a, b } => TextBox {
            left: a.0.min(b.0),
            right: a.0.max(b.0),
            top: a.1.min(b.1),
            bottom: a.1.max(b.1),
        },
        DrawingBodyGeometry::Horizontal { y, x0, x1 } => TextBox {
            // Preserve the semantic direction of a ray. A right ray anchored beyond the pane may
            // intentionally have `left > right`; text placement historically uses that oriented
            // reference rather than normalizing it into a finite segment.
            left: x0,
            right: x1,
            top: y,
            bottom: y,
        },
        DrawingBodyGeometry::Vertical { x, y0, y1 } => TextBox {
            left: x,
            right: x,
            top: y0.min(y1),
            bottom: y0.max(y1),
        },
        DrawingBodyGeometry::Rectangle {
            left,
            right,
            top,
            bottom,
        } => TextBox {
            left,
            right,
            top,
            bottom,
        },
        DrawingBodyGeometry::Polyline { points, .. } => points_box(points)?,
        DrawingBodyGeometry::Position(position) => TextBox {
            left: position.left,
            right: position.right,
            top: position.top(),
            bottom: position.bottom(),
        },
    };
    Some(ResolvedDrawingGeometry { body, text_box })
}
