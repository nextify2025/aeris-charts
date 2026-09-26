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
    /// Up to three independent straight lines (parallel lines and price channels); only the first
    /// `count` are meaningful.
    Lines {
        lines: [((f64, f64), (f64, f64)); 3],
        count: usize,
    },
    /// Fibonacci retracement levels across `x0..x1`: the level at `p` percent sits at
    /// `y0 + (y100 - y0) * p`.
    Fibonacci {
        x0: f64,
        x1: f64,
        y100: f64,
        y0: f64,
    },
    /// A callout above the anchor `(x, y)`: see [`AnnotationGeometry`].
    Annotation(AnnotationGeometry),
}

/// KLineChart's Fibonacci levels, from the first anchor (100%) to the second (0%).
pub(crate) const FIBONACCI_LEVELS: [f64; 7] = [1.0, 0.786, 0.618, 0.5, 0.382, 0.236, 0.0];

impl DrawingBodyGeometry<'_> {
    /// The y of one Fibonacci level.
    pub(crate) fn fibonacci_y(y100: f64, y0: f64, level: f64) -> f64 {
        y0 + (y100 - y0) * level
    }
}

/// KLineChart `simpleAnnotation` in device-independent px, scaled by the device ratio: a stem from
/// 6 px above the anchor rising 50 px, then an 8 px wide arrowhead 5 px tall pointing down at the
/// stem. The label sits above the arrowhead.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnnotationGeometry {
    pub(crate) x: f64,
    /// Stem bottom (6 px above the anchor).
    pub(crate) stem_bottom: f64,
    /// Stem top, where the arrowhead's point meets it.
    pub(crate) stem_top: f64,
    /// Top of the arrowhead, under the label.
    pub(crate) head_top: f64,
    /// Half the arrowhead's width.
    pub(crate) head_half_width: f64,
}

impl AnnotationGeometry {
    fn at(anchor: (f64, f64), scale: f64) -> Self {
        let stem_bottom = anchor.1 - 6.0 * scale;
        let stem_top = stem_bottom - 50.0 * scale;
        Self {
            x: anchor.0,
            stem_bottom,
            stem_top,
            head_top: stem_top - 5.0 * scale,
            head_half_width: 4.0 * scale,
        }
    }
}

/// The two pane-edge ends of the infinite line through `a` and `b` (a vertical line spans the
/// pane height). `None` when the anchors coincide.
fn line_across(
    a: (f64, f64),
    b: (f64, f64),
    pane_w: f64,
    pane_top: f64,
    pane_h: f64,
) -> Option<((f64, f64), (f64, f64))> {
    if (a.0 - b.0).abs() <= f64::EPSILON {
        if (a.1 - b.1).abs() <= f64::EPSILON {
            return None;
        }
        return Some(((a.0, pane_top), (a.0, pane_top + pane_h)));
    }
    let slope = (b.1 - a.1) / (b.0 - a.0);
    let y_at = |x: f64| a.1 + (x - a.0) * slope;
    Some(((0.0, y_at(0.0)), (pane_w, y_at(pane_w))))
}

/// KLineChart `getParallelLines`: the line through the first two points, the parallel through
/// the third, and `extra` more parallels continuing the spacing beyond the first line.
fn parallel_lines(
    px: &[(f64, f64)],
    extra: usize,
    pane_w: f64,
    pane_top: f64,
    pane_h: f64,
) -> DrawingBodyGeometry<'static> {
    let mut lines = [((0.0, 0.0), (0.0, 0.0)); 3];
    let mut count = 0;
    let (Some(&a), Some(&b)) = (px.first(), px.get(1)) else {
        return DrawingBodyGeometry::Lines { lines, count };
    };
    let mut push = |line: ((f64, f64), (f64, f64))| {
        if count < lines.len() {
            lines[count] = line;
            count += 1;
        }
    };
    if (a.0 - b.0).abs() <= f64::EPSILON {
        let vertical = |x: f64| ((x, pane_top), (x, pane_top + pane_h));
        push(vertical(a.0));
        if let Some(&c) = px.get(2) {
            push(vertical(c.0));
            let distance = a.0 - c.0;
            for i in 0..extra {
                push(vertical(a.0 + distance * (i + 1) as f64));
            }
        }
    } else {
        let slope = (b.1 - a.1) / (b.0 - a.0);
        let intercept = a.1 - slope * a.0;
        let line = |intercept: f64| ((0.0, intercept), (pane_w, pane_w * slope + intercept));
        push(line(intercept));
        if let Some(&c) = px.get(2) {
            let parallel = c.1 - slope * c.0;
            push(line(parallel));
            let distance = intercept - parallel;
            for i in 0..extra {
                push(line(intercept + distance * (i + 1) as f64));
            }
        }
    }
    DrawingBodyGeometry::Lines { lines, count }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositionGeometry {
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) entry_y: f64,
    pub(crate) target_y: f64,
    pub(crate) stop_y: f64,
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
        DrawingKind::StraightLine => {
            let (a, b) = line_across(*px.first()?, *px.get(1)?, pane_w, pane_top, pane_h)?;
            DrawingBodyGeometry::Segment { a, b }
        }
        DrawingKind::RayLine => {
            // KLineChart `getRayLine`: toward the second anchor's side, to the pane edge.
            let (a, b) = (*px.first()?, *px.get(1)?);
            let end = if (a.0 - b.0).abs() <= f64::EPSILON {
                if (a.1 - b.1).abs() <= f64::EPSILON {
                    return None;
                }
                (
                    a.0,
                    if a.1 < b.1 {
                        pane_top + pane_h
                    } else {
                        pane_top
                    },
                )
            } else {
                let slope = (b.1 - a.1) / (b.0 - a.0);
                let x = if a.0 > b.0 { 0.0 } else { pane_w };
                (x, a.1 + (x - a.0) * slope)
            };
            DrawingBodyGeometry::Segment { a, b: end }
        }
        DrawingKind::HorizontalSegment => {
            // The anchors share a price; while placing, the newest anchor sets it.
            let (a, b) = (*px.first()?, *px.get(1)?);
            DrawingBodyGeometry::Horizontal {
                y: b.1,
                x0: a.0.min(b.0),
                x1: a.0.max(b.0),
            }
        }
        DrawingKind::VerticalRay => {
            let (a, b) = (*px.first()?, *px.get(1)?);
            DrawingBodyGeometry::Vertical {
                x: b.0,
                y0: a.1,
                y1: if a.1 < b.1 {
                    pane_top + pane_h
                } else {
                    pane_top
                },
            }
        }
        DrawingKind::VerticalSegment => {
            let (a, b) = (*px.first()?, *px.get(1)?);
            DrawingBodyGeometry::Vertical {
                x: b.0,
                y0: a.1,
                y1: b.1,
            }
        }
        DrawingKind::PriceLine => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: px[0].0,
            x1: pane_w,
        },
        DrawingKind::ParallelLine => parallel_lines(px, 0, pane_w, pane_top, pane_h),
        DrawingKind::PriceChannel => parallel_lines(px, 1, pane_w, pane_top, pane_h),
        DrawingKind::FibonacciLine => {
            let (a, b) = (*px.first()?, *px.get(1)?);
            DrawingBodyGeometry::Fibonacci {
                x0: 0.0,
                x1: pane_w,
                y100: a.1,
                y0: b.1,
            }
        }
        DrawingKind::SimpleAnnotation => {
            DrawingBodyGeometry::Annotation(AnnotationGeometry::at(px[0], options.device_scale))
        }
        DrawingKind::SimpleTag => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: 0.0,
            x1: pane_w,
        },
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
        // Channel labels follow the anchors, not the pane-wide lines.
        DrawingBodyGeometry::Lines { .. } => points_box(px)?,
        DrawingBodyGeometry::Fibonacci { x0, x1, y100, y0 } => TextBox {
            left: x0,
            right: x1,
            top: y100.min(y0),
            bottom: y100.max(y0),
        },
        // The label sits on top of the arrowhead.
        DrawingBodyGeometry::Annotation(annotation) => TextBox {
            left: annotation.x,
            right: annotation.x,
            top: annotation.head_top,
            bottom: annotation.head_top,
        },
    };
    Some(ResolvedDrawingGeometry { body, text_box })
}
