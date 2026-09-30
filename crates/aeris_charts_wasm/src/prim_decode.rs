//! Primitive command-buffer decoder (plugin platform Phase C-a).
//!
//! JS pane primitives never touch a canvas: their renderer calls the draw functions on the
//! host-built context, each of which records one plain JS object into a command array. After
//! the renderer returns, the host JSON-stringifies that array (one marshalling pass per
//! primitive per frame) and this module decodes it into the backend-neutral [`Prim`] IR the
//! WebGPU and Canvas2D executors share — so plugin content is pixel-identical across backends
//! by construction.
//!
//! The decoder is pure (no JS, no DOM): it takes the JSON text, the pane's shared point pool and
//! the pane's clip rect, and returns prims plus human-readable warnings for skipped input, so
//! it is fully host-testable. Coordinates arrive in absolute bitmap px (the draw context's
//! converters already applied the pane's pixel ratios and offset); integer prims round here
//! exactly like the engine's own geometry.
//!
//! Executors receive only solid runs from a producer, and the WebGPU stroker has no dash
//! concept, so this decoder is a producer like the engine's frame builders: a dashed or dotted
//! `polyline` is lowered to solid dash runs through
//! [`aeris_charts_render::line::push_styled_stroke`], clipped to the owning pane (the absolute
//! scissor, [`pane_clip`]) with the unclipped dash phase; a dashed `hline`/`vline` is clamped to
//! the pane with the same phase. Clipping to the pane is lossless because both backends already
//! clip plugin layers to that scissor, and it bounds how far a plugin's geometry can reach past
//! the pane (the executors' f32 dash loops never terminate on spans of hundreds of millions of
//! px). Inside the pane the dash count follows the path's visible length, so a dashed polyline
//! whose [`dash_run_bound`] exceeds [`MAX_DASH_RUNS`] is drawn solid with a warning instead. A
//! command may therefore yield zero or many prims.

use aeris_charts_engine::line_style_from_u8;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{Gradient, IRect, LineStyle, LineType, Prim, TextAlign};
use aeris_charts_render::line::{crisp_span, dash_run_bound, push_styled_stroke};
use aeris_charts_render::shape::Rect;

/// Narrowest dashed stroke, in bitmap px. Dash patterns scale with the stroke width, so a
/// vanishing width would explode the run count (or stall the splitter); at this width one
/// straight pane crossing lowers to at most a few hundred runs. Thinner dashed polylines are
/// skipped with a warning; solid ones are unaffected. This bounds the dash rate per px of path;
/// [`MAX_DASH_RUNS`] bounds the total for a path that winds through the pane.
const MIN_DASHED_WIDTH_PX: f64 = 0.5;

/// Most solid dash runs one dashed `polyline` command may lower to. The pane bounds only the
/// reach past its edges: inside it the run count follows the path's visible length
/// ([`dash_run_bound`]), so a dense zigzag of thousands of points would otherwise cost hundreds
/// of thousands of runs, pool points, and Canvas2D strokes every frame (plugin renderers re-run
/// each frame). A command over the budget is drawn as one solid polyline with a warning, which
/// keeps its ink, costs only its own points, and is what the WebGPU stroker painted for a dashed
/// plugin line before lowering. A long dotted line across a wide pane is a few hundred runs, so
/// real strokes sit far below it. The number of commands a buffer holds is the plugin's own cost,
/// as for every other command kind.
const MAX_DASH_RUNS: u32 = 4096;

/// The pane's clip rect for a plugin command buffer: the pane's absolute bitmap-px scissor
/// `[left, top, width, height]` as a [`Rect`]. Plugin coordinates are absolute bitmap px, so this
/// is deliberately not the engine's pane-local rect (which is translated by the pane's left
/// offset only after frame assembly).
pub fn pane_clip(scissor: [u32; 4]) -> Rect {
    let [left, top, width, height] = scissor.map(f64::from);
    Rect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

/// Defaults the host folds into `text` commands (the draw context has no font state of its
/// own): the layout font family, the layout font size scaled to the pane's bitmap px, and the
/// layout text color — the same sources the engine's own axis labels draw with.
#[derive(Clone, Debug)]
pub struct TextDefaults {
    pub family: String,
    /// Bitmap px (the command coordinate space): `layout.fontSize × dpr`.
    pub size: f32,
    pub color: Color,
}

/// Decode result: the prims produced (in command order) plus one warning per skipped command.
#[derive(Clone, Debug, Default)]
pub struct DecodedCommands {
    pub prims: Vec<Prim>,
    pub warnings: Vec<String>,
}

impl DecodedCommands {
    fn warn(&mut self, index: usize, detail: String) {
        self.warnings.push(format!("command {index}: {detail}"));
    }
}

fn num(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64().filter(|v| v.is_finite())
}

fn color(value: &serde_json::Value, key: &str) -> Option<Color> {
    Color::parse_css(value.get(key)?.as_str()?)
}

/// Optional CSS color slot (`undefined`/`null`/absent → None; unparseable → None).
fn optional_color(value: &serde_json::Value, key: &str) -> Option<Color> {
    value.get(key)?.as_str().and_then(Color::parse_css)
}

fn style(value: &serde_json::Value) -> aeris_charts_render::draw_list::LineStyle {
    line_style_from_u8(num(value, "style").unwrap_or(0.0).clamp(0.0, 255.0) as u8)
}

/// Walk a flat `[x0,y0,x1,y1,...]` JSON array (a trailing odd value is dropped), calling `emit`
/// with each pair. `None` when the array is missing or holds a non-number or non-finite value;
/// `emit` has then already seen the leading pairs, so callers discard what they collected.
fn for_each_point(value: &serde_json::Value, mut emit: impl FnMut(f64, f64)) -> Option<usize> {
    let flat = value.get("points")?.as_array()?;
    let mut count = 0;
    for pair in flat.as_chunks::<2>().0 {
        let (x, y) = (pair[0].as_f64()?, pair[1].as_f64()?);
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        emit(x, y);
        count += 1;
    }
    Some(count)
}

/// Push `count` points from a flat `[x0,y0,x1,y1,...]` JSON array into the shared pool.
/// Returns the `(first_point, point_count)` window, or `None` when the array is malformed or
/// holds fewer than two points (both executors ignore degenerate runs, but skipping keeps the
/// pool clean).
fn push_points(value: &serde_json::Value, pool: &mut Vec<[f32; 2]>) -> Option<(u32, u32)> {
    let first = pool.len();
    let count = for_each_point(value, |x, y| pool.push([x as f32, y as f32]));
    match count {
        Some(count) if count >= 2 => Some((first as u32, count as u32)),
        _ => {
            pool.truncate(first);
            None
        }
    }
}

const POINTS_ERROR: &str = "points (need a flat [x,y,...] array of 2+ points)";

/// A crisp line's `[from, to]` extent in whole px. A solid line is one bounded rect and passes
/// through unchanged. A dashed one is clamped to `bounds` (the pane) keeping the dash phase; an
/// empty or reversed extent (which every executor draws as nothing) and one that misses the pane
/// (which the pane's scissor would clip away) yield `None`.
fn crisp_extent(
    from: f64,
    to: f64,
    bounds: (f64, f64),
    width: i32,
    style: LineStyle,
) -> Option<(i32, i32)> {
    if style == LineStyle::Solid {
        return Some((from as i32, to as i32));
    }
    if from >= to {
        return None;
    }
    crisp_span(from, to, bounds, width, style)
}

fn decode_polyline(
    command: &serde_json::Value,
    index: usize,
    pool: &mut Vec<[f32; 2]>,
    pane: Rect,
    out: &mut DecodedCommands,
) -> Result<(), String> {
    let width = num(command, "width").ok_or("width")?;
    let style = style(command);
    let color = color(command, "color").ok_or("color")?;
    if style == LineStyle::Solid {
        let (first_point, point_count) = push_points(command, pool).ok_or(POINTS_ERROR)?;
        out.prims.push(Prim::Polyline {
            first_point,
            point_count,
            width: width as f32,
            style,
            line_type: LineType::Simple,
            color,
        });
        return Ok(());
    }
    if width < MIN_DASHED_WIDTH_PX {
        return Err(format!(
            "dashed polyline width {width} is below {MIN_DASHED_WIDTH_PX} px"
        ));
    }
    let mut run = Vec::new();
    for_each_point(command, |x, y| run.push((x, y)))
        .filter(|&count| count >= 2)
        .ok_or(POINTS_ERROR)?;
    let width = width as f32;
    let bound = dash_run_bound(&run, pane, width, style);
    // A NaN bound compares false, so it also takes the solid path.
    let style = if bound <= f64::from(MAX_DASH_RUNS) {
        style
    } else {
        out.warn(
            index,
            format!(
                "dashed polyline could lower to {bound:.0} dash runs, over the budget of \
                 {MAX_DASH_RUNS}; drawn solid"
            ),
        );
        LineStyle::Solid
    };
    // Zero prims (a run wholly outside the pane) is not an error.
    push_styled_stroke(
        &mut out.prims,
        pool,
        &run,
        LineType::Simple,
        (width, style, color),
        pane,
    );
    Ok(())
}

/// Decode the command at `index`, appending its prims (zero, one, or many) to `out`.
fn decode_one(
    command: &serde_json::Value,
    index: usize,
    pool: &mut Vec<[f32; 2]>,
    text_defaults: &TextDefaults,
    pane: Rect,
    out: &mut DecodedCommands,
) -> Result<(), String> {
    let kind = command
        .get("c")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing \"c\" kind".to_string())?;
    match kind {
        "hline" => {
            let y = num(command, "y").ok_or("y")?.round() as i32;
            let x0 = num(command, "x1").ok_or("x1")?.round();
            let x1 = num(command, "x2").ok_or("x2")?.round();
            let width = num(command, "width").ok_or("width")?.round().max(1.0) as i32;
            let style = style(command);
            let color = color(command, "color").ok_or("color")?;
            if let Some((x0, x1)) = crisp_extent(x0, x1, (pane.left, pane.right), width, style) {
                out.prims.push(Prim::HLine {
                    y,
                    x0,
                    x1,
                    width,
                    style,
                    color,
                });
            }
            Ok(())
        }
        "vline" => {
            let x = num(command, "x").ok_or("x")?.round() as i32;
            let y0 = num(command, "y1").ok_or("y1")?.round();
            let y1 = num(command, "y2").ok_or("y2")?.round();
            let width = num(command, "width").ok_or("width")?.round().max(1.0) as i32;
            let style = style(command);
            let color = color(command, "color").ok_or("color")?;
            if let Some((y0, y1)) = crisp_extent(y0, y1, (pane.top, pane.bottom), width, style) {
                out.prims.push(Prim::VLine {
                    x,
                    y0,
                    y1,
                    width,
                    style,
                    color,
                });
            }
            Ok(())
        }
        "polyline" => decode_polyline(command, index, pool, pane, out),
        _ => {
            out.prims
                .push(decode_prim(kind, command, pool, text_defaults)?);
            Ok(())
        }
    }
}

/// Decode the commands that map to exactly one prim.
fn decode_prim(
    kind: &str,
    command: &serde_json::Value,
    pool: &mut Vec<[f32; 2]>,
    text_defaults: &TextDefaults,
) -> Result<Prim, String> {
    match kind {
        "rect" => Ok(Prim::Rect {
            rect: IRect {
                x: num(command, "x").ok_or("x")?.round() as i32,
                y: num(command, "y").ok_or("y")?.round() as i32,
                w: num(command, "w").ok_or("w")?.round() as i32,
                h: num(command, "h").ok_or("h")?.round() as i32,
            },
            color: color(command, "color").ok_or("color")?,
        }),
        "rect_frame" => Ok(Prim::RectFrame {
            rect: IRect {
                x: num(command, "x").ok_or("x")?.round() as i32,
                y: num(command, "y").ok_or("y")?.round() as i32,
                w: num(command, "w").ok_or("w")?.round() as i32,
                h: num(command, "h").ok_or("h")?.round() as i32,
            },
            border: num(command, "line_width")
                .ok_or("line_width")?
                .round()
                .max(1.0) as i32,
            color: color(command, "color").ok_or("color")?,
        }),
        "area_fill" => {
            let (first_point, point_count) = push_points(command, pool).ok_or(POINTS_ERROR)?;
            Ok(Prim::AreaFill {
                first_point,
                point_count,
                base_y: num(command, "base_y").ok_or("base_y")? as f32,
                line_type: LineType::Simple,
                gradient: Gradient {
                    top: color(command, "top_color").ok_or("top_color")?,
                    bottom: color(command, "bottom_color").ok_or("bottom_color")?,
                },
            })
        }
        "circle" => Ok(Prim::Circle {
            cx: num(command, "x").ok_or("x")? as f32,
            cy: num(command, "y").ok_or("y")? as f32,
            radius: num(command, "r").ok_or("r")? as f32,
            fill: color(command, "fill_color").ok_or("fill_color")?,
            stroke_width: num(command, "border_width").unwrap_or(0.0) as f32,
            stroke: optional_color(command, "border_color").unwrap_or(Color::rgba(0, 0, 0, 0)),
        }),
        "round_rect" => {
            let r = num(command, "r").ok_or("r")? as f32;
            Ok(Prim::RoundRect {
                x: num(command, "x").ok_or("x")? as f32,
                y: num(command, "y").ok_or("y")? as f32,
                w: num(command, "w").ok_or("w")? as f32,
                h: num(command, "h").ok_or("h")? as f32,
                radii: [r, r, r, r],
                fill: color(command, "color").ok_or("color")?,
                border_width: 0.0,
                border_color: Color::rgba(0, 0, 0, 0),
            })
        }
        "triangle" => Ok(Prim::Triangle {
            a: [
                num(command, "x1").ok_or("x1")? as f32,
                num(command, "y1").ok_or("y1")? as f32,
            ],
            b: [
                num(command, "x2").ok_or("x2")? as f32,
                num(command, "y2").ok_or("y2")? as f32,
            ],
            c: [
                num(command, "x3").ok_or("x3")? as f32,
                num(command, "y3").ok_or("y3")? as f32,
            ],
            color: color(command, "color").ok_or("color")?,
        }),
        // Text runs decode fully: the anchor (x = aligned edge, y = vertical center — the
        // axis labels' middle-baseline convention), the run, and the font components, with the
        // host's layout defaults folded in so the prim is backend-ready. Both backends paint
        // it in layer order through the browser's text engine (Canvas2D `fillText` directly,
        // WebGPU via a host-rasterized atlas quad of the same run).
        "text" => Ok(Prim::Text {
            x: num(command, "x").ok_or("x")? as f32,
            y: num(command, "y").ok_or("y")? as f32,
            text: command
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            color: optional_color(command, "color").unwrap_or(text_defaults.color),
            size: num(command, "size")
                .filter(|s| *s > 0.0)
                .unwrap_or(f64::from(text_defaults.size)) as f32,
            family: command
                .get("font")
                .and_then(serde_json::Value::as_str)
                .filter(|f| !f.is_empty())
                .unwrap_or(&text_defaults.family)
                .to_string(),
            align: match command.get("align").and_then(serde_json::Value::as_str) {
                Some("center") => TextAlign::Center,
                Some("right") => TextAlign::Right,
                _ => TextAlign::Left,
            },
            // Numeric CSS weight (100–900) wins; the boolean `bold` shorthand maps to 700.
            weight: command
                .get("weight")
                .and_then(serde_json::Value::as_u64)
                .map(|w| (w as u16).clamp(100, 900))
                .unwrap_or(
                    if command
                        .get("bold")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        700
                    } else {
                        400
                    },
                ),
            italic: command
                .get("italic")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        }),
        other => Err(format!("unknown command {other:?}")),
    }
}

/// Decode one renderer's JSON command array into prims, appending polyline/area points to
/// `pool`. `pane` is the owning pane's clip rect in the commands' absolute bitmap px
/// ([`pane_clip`]); dashed strokes are lowered to solid runs bounded by it. Malformed JSON,
/// non-array input, and per-command problems skip with a warning rather than failing the frame —
/// a broken plugin must never take the chart down.
pub fn decode_commands(
    json: &str,
    pool: &mut Vec<[f32; 2]>,
    text_defaults: &TextDefaults,
    pane: Rect,
) -> DecodedCommands {
    let mut out = DecodedCommands::default();
    let parsed = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => value,
        Err(error) => {
            out.warnings
                .push(format!("malformed command buffer: {error}"));
            return out;
        }
    };
    let Some(commands) = parsed.as_array() else {
        out.warnings
            .push("command buffer is not an array".to_string());
        return out;
    };
    for (index, command) in commands.iter().enumerate() {
        if let Err(detail) = decode_one(command, index, pool, text_defaults, pane, &mut out) {
            out.warn(index, format!("skipped ({detail})"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_charts_render::draw_list::LineStyle;

    /// A pane far larger than any coordinate the general decode tests use, so their prims are
    /// never clipped.
    const WIDE_PANE: Rect = Rect {
        left: -1.0e6,
        top: -1.0e6,
        right: 1.0e6,
        bottom: 1.0e6,
    };

    fn defaults() -> TextDefaults {
        TextDefaults {
            family: "DefaultFamily".into(),
            size: 24.0,
            color: Color::rgb(0x11, 0x22, 0x33),
        }
    }

    fn decode_in(json: &str, pane: Rect) -> (Vec<Prim>, Vec<[f32; 2]>, Vec<String>) {
        let mut pool = Vec::new();
        let out = decode_commands(json, &mut pool, &defaults(), pane);
        (out.prims, pool, out.warnings)
    }

    fn decode(json: &str) -> (Vec<Prim>, Vec<[f32; 2]>, Vec<String>) {
        decode_in(json, WIDE_PANE)
    }

    #[test]
    fn rect_and_rect_frame_decode_to_integer_prims() {
        let (prims, _, warnings) = decode(
            r##"[
                {"c":"rect","x":10.4,"y":20.6,"w":100.0,"h":50.0,"color":"#ff0000"},
                {"c":"rect_frame","x":1.0,"y":2.0,"w":30.0,"h":40.0,"color":"rgba(0,128,0,0.5)","line_width":2.0}
            ]"##,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            prims,
            vec![
                Prim::Rect {
                    rect: IRect {
                        x: 10,
                        y: 21,
                        w: 100,
                        h: 50
                    },
                    color: Color::rgb(0xff, 0x00, 0x00),
                },
                Prim::RectFrame {
                    rect: IRect {
                        x: 1,
                        y: 2,
                        w: 30,
                        h: 40
                    },
                    border: 2,
                    color: Color::rgba(0, 128, 0, 128),
                },
            ]
        );
    }

    #[test]
    fn hline_and_vline_decode_with_style_and_min_width() {
        let (prims, _, warnings) = decode(
            r##"[
                {"c":"hline","y":50.5,"x1":0.0,"x2":640.0,"color":"#2196f3","width":0.0,"style":2},
                {"c":"vline","x":100.0,"y1":10.0,"y2":490.0,"color":"#9598a1","width":2.0,"style":4}
            ]"##,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            prims,
            vec![
                Prim::HLine {
                    y: 51,
                    x0: 0,
                    x1: 640,
                    width: 1,
                    style: LineStyle::Dashed,
                    color: Color::rgb(0x21, 0x96, 0xf3),
                },
                Prim::VLine {
                    x: 100,
                    y0: 10,
                    y1: 490,
                    width: 2,
                    style: LineStyle::Dotted,
                    color: Color::rgb(0x95, 0x98, 0xa1),
                },
            ]
        );
    }

    #[test]
    fn polyline_appends_points_to_the_shared_pool() {
        // The second polyline is dotted, which the decoder lowers (a dotted producer must hand
        // executors solid runs, see `dashed_polyline_lowers_to_solid_dash_runs`): at width 1 the
        // dotted pattern is [1, 4], so the (1,1)->(2,2) diagonal keeps one 1 px dot and drops
        // the rest of its ~1.41 px length into the gap. The solid polyline is pooled verbatim.
        let (prims, pool, warnings) = decode(
            r##"[
                {"c":"polyline","points":[0,0, 10.5,20.25, 30,40],"color":"#0000ff","width":2.5,"style":0},
                {"c":"polyline","points":[1,1, 2,2],"color":"#0000ff","width":1.0,"style":1}
            ]"##,
        );
        assert!(warnings.is_empty());
        let dot_end = (1.0 + std::f64::consts::FRAC_1_SQRT_2) as f32;
        assert_eq!(
            pool,
            vec![
                [0.0, 0.0],
                [10.5, 20.25],
                [30.0, 40.0],
                [1.0, 1.0],
                [dot_end, dot_end]
            ]
        );
        assert_eq!(
            prims,
            vec![
                Prim::Polyline {
                    first_point: 0,
                    point_count: 3,
                    width: 2.5,
                    style: LineStyle::Solid,
                    line_type: LineType::Simple,
                    color: Color::rgb(0x00, 0x00, 0xff),
                },
                Prim::Polyline {
                    first_point: 3,
                    point_count: 2,
                    width: 1.0,
                    style: LineStyle::Solid,
                    line_type: LineType::Simple,
                    color: Color::rgb(0x00, 0x00, 0xff),
                },
            ]
        );
    }

    #[test]
    fn area_fill_decodes_gradient_and_base() {
        let (prims, pool, warnings) = decode(
            r##"[{"c":"area_fill","points":[0,10, 20,30],"base_y":50.5,"top_color":"#2edc8766","bottom_color":"rgba(40,221,100,0)"}]"##,
        );
        assert!(warnings.is_empty());
        assert_eq!(pool, vec![[0.0, 10.0], [20.0, 30.0]]);
        assert_eq!(
            prims,
            vec![Prim::AreaFill {
                first_point: 0,
                point_count: 2,
                base_y: 50.5,
                line_type: LineType::Simple,
                gradient: Gradient {
                    top: Color::rgba(0x2e, 0xdc, 0x87, 0x66),
                    bottom: Color::rgba(40, 221, 100, 0),
                },
            }]
        );
    }

    #[test]
    fn circle_round_rect_triangle_and_text_decode() {
        let (prims, _, warnings) = decode(
            r##"[
                {"c":"circle","x":100.0,"y":50.0,"r":8.0,"fill_color":"#ff0000","border_color":"#000000","border_width":2.0},
                {"c":"circle","x":1.0,"y":2.0,"r":3.0,"fill_color":"#00ff00"},
                {"c":"round_rect","x":10.0,"y":20.0,"w":80.0,"h":30.0,"r":6.0,"color":"#123456"},
                {"c":"triangle","x1":0.0,"y1":0.0,"x2":10.0,"y2":0.0,"x3":5.0,"y3":8.0,"color":"#abcdef"},
                {"c":"text","x":40.0,"y":60.0,"text":"hello","color":"#191919","size":12,"align":"center","bold":true}
            ]"##,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            prims,
            vec![
                Prim::Circle {
                    cx: 100.0,
                    cy: 50.0,
                    radius: 8.0,
                    fill: Color::rgb(0xff, 0x00, 0x00),
                    stroke_width: 2.0,
                    stroke: Color::rgb(0, 0, 0),
                },
                Prim::Circle {
                    cx: 1.0,
                    cy: 2.0,
                    radius: 3.0,
                    fill: Color::rgb(0x00, 0xff, 0x00),
                    stroke_width: 0.0,
                    stroke: Color::rgba(0, 0, 0, 0),
                },
                Prim::RoundRect {
                    x: 10.0,
                    y: 20.0,
                    w: 80.0,
                    h: 30.0,
                    radii: [6.0, 6.0, 6.0, 6.0],
                    fill: Color::rgb(0x12, 0x34, 0x56),
                    border_width: 0.0,
                    border_color: Color::rgba(0, 0, 0, 0),
                },
                Prim::Triangle {
                    a: [0.0, 0.0],
                    b: [10.0, 0.0],
                    c: [5.0, 8.0],
                    color: Color::rgb(0xab, 0xcd, 0xef),
                },
                Prim::Text {
                    x: 40.0,
                    y: 60.0,
                    text: "hello".into(),
                    color: Color::rgb(0x19, 0x19, 0x19),
                    size: 12.0,
                    family: "DefaultFamily".into(),
                    align: TextAlign::Center,
                    weight: 700,
                    italic: false,
                },
            ]
        );
    }

    #[test]
    fn text_decodes_defaults_and_explicit_font_family() {
        let (prims, _, warnings) = decode(
            r##"[
                {"c":"text","x":1.0,"y":2.0,"text":"bare"},
                {"c":"text","x":3.0,"y":4.0,"text":"styled","size":9.5,"font":"Custom","align":"right","bold":true,"color":"#ff0000"},
                {"c":"text","x":5.0,"y":6.0,"text":"badalign","align":"justify"},
                {"c":"text","x":7.0,"y":8.0,"text":"badsize","size":0}
            ]"##,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            prims,
            vec![
                Prim::Text {
                    x: 1.0,
                    y: 2.0,
                    text: "bare".into(),
                    color: Color::rgb(0x11, 0x22, 0x33),
                    size: 24.0,
                    family: "DefaultFamily".into(),
                    align: TextAlign::Left,
                    weight: 400,
                    italic: false,
                },
                Prim::Text {
                    x: 3.0,
                    y: 4.0,
                    text: "styled".into(),
                    color: Color::rgb(0xff, 0x00, 0x00),
                    size: 9.5,
                    family: "Custom".into(),
                    align: TextAlign::Right,
                    weight: 700,
                    italic: false,
                },
                Prim::Text {
                    x: 5.0,
                    y: 6.0,
                    text: "badalign".into(),
                    color: Color::rgb(0x11, 0x22, 0x33),
                    size: 24.0,
                    family: "DefaultFamily".into(),
                    align: TextAlign::Left,
                    weight: 400,
                    italic: false,
                },
                Prim::Text {
                    x: 7.0,
                    y: 8.0,
                    text: "badsize".into(),
                    color: Color::rgb(0x11, 0x22, 0x33),
                    size: 24.0,
                    family: "DefaultFamily".into(),
                    align: TextAlign::Left,
                    weight: 400,
                    italic: false,
                },
            ]
        );
    }

    #[test]
    fn unknown_commands_are_skipped_with_a_warning() {
        let (prims, _, warnings) = decode(
            r##"[
                {"c":"rect","x":0,"y":0,"w":10,"h":10,"color":"#000000"},
                {"c":"sparkle","x":1},
                {"c":"hline","y":5,"x1":0,"x2":10,"color":"#000000","width":1,"style":0}
            ]"##,
        );
        assert_eq!(prims.len(), 2);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unknown command"));
        assert!(warnings[0].contains("command 1"));
    }

    #[test]
    fn malformed_input_never_panics() {
        // Not JSON at all.
        let (prims, _, warnings) = decode("not json");
        assert!(prims.is_empty());
        assert_eq!(warnings.len(), 1);
        // Valid JSON but not an array.
        for input in ["{}", "42", "\"text\"", "null"] {
            let (prims, _, warnings) = decode(input);
            assert!(prims.is_empty(), "input {input}");
            assert_eq!(warnings.len(), 1, "input {input}");
        }
        // Missing fields, wrong types, non-finite-as-null, odd point arrays, too few points.
        let (prims, pool, warnings) = decode(
            r##"[
                {"c":"rect"},
                {"c":"rect","x":"10","y":0,"w":10,"h":10,"color":"#000"},
                {"c":"rect","x":null,"y":0,"w":10,"h":10,"color":"#000"},
                {"c":"rect","x":0,"y":0,"w":10,"h":10,"color":"not-a-color"},
                {"c":"hline","y":5,"x1":0,"x2":10,"color":"#000"},
                {"c":"polyline","points":[0,0,1],"color":"#000","width":1,"style":0},
                {"c":"polyline","points":[0,0],"color":"#000","width":1,"style":0},
                {"c":"polyline","points":"nope","color":"#000","width":1,"style":0},
                {"c":"circle","x":0,"y":0,"fill_color":"#000"},
                {"c":"text","x":0,"y":0},
                {"c":"rect","x":0,"y":0,"w":10,"h":10,"color":"#000000"}
            ]"##,
        );
        // Only the final well-formed rect and the defaulted text survive.
        assert_eq!(prims.len(), 2);
        assert!(matches!(prims[0], Prim::Text { .. }));
        assert!(matches!(prims[1], Prim::Rect { .. }));
        assert!(pool.is_empty());
        assert_eq!(warnings.len(), 9);
    }

    #[test]
    fn pool_is_rolled_back_when_a_point_run_is_malformed() {
        let mut pool = vec![[9.0, 9.0]];
        let out = decode_commands(
            r##"[
                {"c":"polyline","points":[0,0, 1,1],"color":"#000","width":1,"style":0},
                {"c":"polyline","points":[2,2, "bad",3],"color":"#000","width":1,"style":0}
            ]"##,
            &mut pool,
            &defaults(),
            WIDE_PANE,
        );
        assert_eq!(out.prims.len(), 1);
        assert_eq!(out.warnings.len(), 1);
        // The failed run must not leave half-pushed points in the pool.
        assert_eq!(pool, vec![[9.0, 9.0], [0.0, 0.0], [1.0, 1.0]]);
    }

    /// A 400x100 pane whose top-left corner is (0, 0).
    const PANE: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 400.0,
        bottom: 100.0,
    };

    /// Every prim as a solid, simple polyline run (the lowering contract executors rely on),
    /// resolved against the pool.
    fn solid_runs(prims: &[Prim], pool: &[[f32; 2]]) -> Vec<Vec<[f32; 2]>> {
        prims
            .iter()
            .map(|prim| {
                let Prim::Polyline {
                    first_point,
                    point_count,
                    style,
                    line_type,
                    ..
                } = *prim
                else {
                    panic!("expected only polylines, got {prim:?}");
                };
                assert_eq!(style, LineStyle::Solid, "executors receive solid runs");
                assert_eq!(line_type, LineType::Simple);
                pool[first_point as usize..(first_point + point_count) as usize].to_vec()
            })
            .collect()
    }

    /// x extents of a horizontal set of runs.
    fn x_spans(runs: &[Vec<[f32; 2]>]) -> Vec<(f32, f32)> {
        runs.iter()
            .map(|run| (run[0][0], run[run.len() - 1][0]))
            .collect()
    }

    /// Dash spans `[start + k*period, start + k*period + on]` clipped at `end`.
    fn dash_spans(start: f32, end: f32, on: f32, period: f32) -> Vec<(f32, f32)> {
        let mut spans = Vec::new();
        let mut at = start;
        while at < end {
            spans.push((at, (at + on).min(end)));
            at += period;
        }
        spans
    }

    #[test]
    fn dashed_polyline_lowers_to_solid_dash_runs() {
        // Width 2: Dashed is [12, 12] (styles 2 and 3), Dotted is [2, 8] (styles 1 and 4).
        for (style, on, period) in [
            (2, 12.0, 24.0),
            (3, 12.0, 24.0),
            (1, 2.0, 10.0),
            (4, 2.0, 10.0),
        ] {
            let (prims, pool, warnings) = decode_in(
                &format!(
                    r##"[{{"c":"polyline","points":[0,50,200,50],"color":"#0000ff","width":2,"style":{style}}}]"##
                ),
                PANE,
            );
            assert!(warnings.is_empty(), "style {style}: {warnings:?}");
            assert!(prims.len() > 1, "style {style} splits into dashes");
            let runs = solid_runs(&prims, &pool);
            assert_eq!(
                x_spans(&runs),
                dash_spans(0.0, 200.0, on, period),
                "style {style}"
            );
            assert!(runs.iter().flatten().all(|p| p[1] == 50.0));
            assert!(prims.iter().all(|prim| matches!(
                prim,
                Prim::Polyline { width, color, .. }
                    if *width == 2.0 && *color == Color::rgb(0, 0, 0xff)
            )));
        }
    }

    #[test]
    fn dashed_polyline_keeps_command_order_and_the_unclipped_diagonal_phase() {
        // A dotted diagonal that starts left of the pane and leaves past its right edge, between
        // two rects: prims stay in command order, and the dots inside the pane are exactly the
        // ones the unclipped polyline would paint.
        let (prims, pool, warnings) = decode_in(
            r##"[
                {"c":"rect","x":0,"y":0,"w":5,"h":5,"color":"#111111"},
                {"c":"polyline","points":[-300,-100, 700,300],"color":"#ff0000","width":3,"style":1},
                {"c":"rect","x":9,"y":9,"w":5,"h":5,"color":"#222222"}
            ]"##,
            PANE,
        );
        assert!(warnings.is_empty());
        assert!(matches!(prims.first(), Some(Prim::Rect { .. })));
        assert!(matches!(prims.last(), Some(Prim::Rect { .. })));
        let runs = solid_runs(&prims[1..prims.len() - 1], &pool);
        assert!(runs.len() > 10);

        let expanded = [
            aeris_charts_render::line::LinePoint {
                x: -300.0,
                y: -100.0,
            },
            aeris_charts_render::line::LinePoint { x: 700.0, y: 300.0 },
        ];
        let pattern: Vec<f64> = LineStyle::Dotted
            .dash_pattern(3.0)
            .iter()
            .map(|&len| f64::from(len))
            .collect();
        let inside = |p: (f64, f64)| PANE.contains(p);
        let reference: Vec<[(f64, f64); 2]> =
            aeris_charts_render::line::dash_split(&expanded, &pattern)
                .iter()
                .map(|run| {
                    let (a, b) = (run[0], run[run.len() - 1]);
                    [(a.x, a.y), (b.x, b.y)]
                })
                .filter(|[a, b]| inside(*a) && inside(*b))
                .collect();
        let decoded: Vec<[(f64, f64); 2]> = runs
            .iter()
            .map(|run| {
                let (a, b) = (run[0], run[run.len() - 1]);
                [
                    (f64::from(a[0]), f64::from(a[1])),
                    (f64::from(b[0]), f64::from(b[1])),
                ]
            })
            .filter(|[a, b]| inside(*a) && inside(*b))
            .collect();
        assert!(!reference.is_empty());
        assert_eq!(decoded.len(), reference.len());
        for (got, want) in decoded.iter().zip(&reference) {
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g.0 - w.0).abs() < 1e-2 && (g.1 - w.1).abs() < 1e-2,
                    "{got:?} vs {want:?}"
                );
            }
        }
    }

    #[test]
    fn dashed_polyline_outside_the_pane_is_dropped_without_a_warning() {
        let (prims, pool, warnings) = decode_in(
            r##"[
                {"c":"polyline","points":[0,500,300,500],"color":"#000","width":2,"style":2},
                {"c":"polyline","points":[900,10,600,90],"color":"#000","width":2,"style":1}
            ]"##,
            PANE,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(prims.is_empty());
        assert!(pool.is_empty());
    }

    #[test]
    fn decoded_dashed_polyline_has_gaps_on_the_webgpu_tessellator() {
        // The WebGPU stroker ignores `Prim::Polyline::style`, so gapped ink only exists when the
        // decoder pre-lowered the dashes: no triangle may reach across a gap.
        let (prims, pool, warnings) = decode_in(
            r##"[{"c":"polyline","points":[0,50,200,50],"color":"#0000ff","width":2,"style":2}]"##,
            PANE,
        );
        assert!(warnings.is_empty());
        let mut vertices = Vec::new();
        for prim in &prims {
            aeris_charts_render_wgpu::geom_prim_to_tris(prim, &pool, &mut vertices);
        }
        assert!(!vertices.is_empty());
        let spans = dash_spans(0.0, 200.0, 12.0, 24.0);
        // Fully covered vertices sit exactly at the dash ends: two per dash.
        let mut solid_x: Vec<f32> = vertices
            .iter()
            .filter(|v| v.color[3] == 1.0)
            .map(|v| v.pos[0])
            .collect();
        solid_x.sort_by(f32::total_cmp);
        solid_x.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        assert_eq!(solid_x.len(), spans.len() * 2, "{solid_x:?}");
        for (pair, span) in solid_x.chunks(2).zip(&spans) {
            assert!((pair[0] - span.0).abs() <= 1.0 && (pair[1] - span.1).abs() <= 1.0);
        }
        // Every painted triangle lies within one dash (plus its half-pixel anti-aliased cap).
        for triangle in vertices.chunks(3) {
            let low = triangle
                .iter()
                .map(|v| v.pos[0])
                .fold(f32::INFINITY, f32::min);
            let high = triangle
                .iter()
                .map(|v| v.pos[0])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                spans.iter().any(|s| low >= s.0 - 1.0 && high <= s.1 + 1.0),
                "triangle spans a gap: {low}..{high}"
            );
        }
    }

    #[test]
    fn dashed_polyline_reaching_far_past_the_pane_stays_bounded_and_in_phase() {
        // Period 24; -24000 is a whole number of periods, so dashes start at multiples of 24.
        let (prims, pool, warnings) = decode_in(
            r##"[{"c":"polyline","points":[-24000,50,1000000000,50],"color":"#000","width":2,"style":2}]"##,
            PANE,
        );
        assert!(warnings.is_empty());
        assert!(prims.len() <= 400 / 24 + 4, "{} prims", prims.len());
        assert!(pool.len() <= prims.len() * 2);
        let reach = PANE.inflate(2.0 + 2.0 + 24.0);
        assert!(pool
            .iter()
            .all(|p| f64::from(p[0]) >= reach.left && f64::from(p[0]) <= reach.right));
        let spans = x_spans(&solid_runs(&prims, &pool));
        assert!(
            spans.iter().all(|s| s.0.rem_euclid(24.0) == 0.0),
            "{spans:?}"
        );
        let first_visible = spans.iter().find(|s| s.1 > 0.0).unwrap();
        assert_eq!(*first_visible, (0.0, 12.0));
    }

    #[test]
    fn dashed_polyline_below_the_minimum_width_is_skipped_with_a_warning() {
        let mut pool = vec![[9.0, 9.0]];
        let out = decode_commands(
            r##"[
                {"c":"polyline","points":[0,50,300,50],"color":"#000","width":0,"style":2},
                {"c":"polyline","points":[0,50,300,50],"color":"#000","width":-1,"style":1},
                {"c":"polyline","points":[0,50,300,50],"color":"#000","width":0.001,"style":2},
                {"c":"polyline","points":[0,50,300,50],"color":"#000","width":1e-15,"style":4},
                {"c":"polyline","points":[0,50,300,50],"color":"#000","width":0.499,"style":3}
            ]"##,
            &mut pool,
            &defaults(),
            PANE,
        );
        assert!(out.prims.is_empty());
        assert_eq!(out.warnings.len(), 5, "{:?}", out.warnings);
        assert!(out.warnings[2].contains("command 2") && out.warnings[2].contains("width"));
        assert_eq!(pool, vec![[9.0, 9.0]]);

        // The narrowest accepted dashed width still lowers to a bounded run count.
        let (prims, _, warnings) = decode_in(
            r##"[{"c":"polyline","points":[0,50,400,50],"color":"#000","width":0.5,"style":1}]"##,
            PANE,
        );
        assert!(warnings.is_empty());
        assert!(prims.len() <= 400, "{} runs", prims.len());

        // A solid polyline of the same tiny width is untouched.
        let (prims, pool, warnings) = decode(
            r##"[{"c":"polyline","points":[0,50,300,50],"color":"#000","width":0.001,"style":0}]"##,
        );
        assert!(warnings.is_empty());
        assert_eq!(prims.len(), 1);
        assert_eq!(pool, vec![[0.0, 50.0], [300.0, 50.0]]);
    }

    /// A flat `[x,y,...]` zigzag of `count` points sweeping the pane's height, `step` px apart in
    /// x: each segment crosses the whole 100 px pane height, so the in-pane path length is about
    /// `count * 100` px however small the x extent is.
    fn zigzag_points(count: usize, step: f64) -> String {
        (0..count)
            .map(|i| format!("{},{}", i as f64 * step, if i % 2 == 0 { 0 } else { 100 }))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn zigzag_command(count: usize, width: f64, style: u8) -> String {
        format!(
            r##"{{"c":"polyline","points":[{}],"color":"#0000ff","width":{width},"style":{style}}}"##,
            zigzag_points(count, 0.2)
        )
    }

    #[test]
    fn dense_dashed_polyline_inside_the_pane_is_bounded_and_drawn_solid() {
        // 2,000 points of 100 px each inside a 400x100 pane is ~200,000 px of visible dotted path:
        // ~20,000 dash runs at width 2 (period 10) and ~80,000 at width 0.5. Unbounded, this is
        // per-frame work and memory proportional to the plugin's path length, not to the pane.
        for (width, style) in [(2.0, 1), (2.0, 2), (0.5, 1), (3.0, 4)] {
            let (prims, pool, warnings) = decode_in(
                &format!(
                    "[{}, {}]",
                    zigzag_command(2_000, width, style),
                    zigzag_command(50, 2.0, 0)
                ),
                PANE,
            );
            assert!(
                prims.len() <= MAX_DASH_RUNS as usize + 1,
                "width {width} style {style}: {} prims",
                prims.len()
            );
            assert!(
                pool.len() <= 2 * (MAX_DASH_RUNS as usize + 1) + 2_000,
                "{} pool points",
                pool.len()
            );
            // The over-budget command keeps its ink as one solid polyline (the WebGPU stroker
            // already painted dashed plugin lines solid); the solid command after it is untouched.
            assert_eq!(prims.len(), 2, "width {width} style {style}");
            let runs = solid_runs(&prims, &pool);
            assert_eq!((runs[0].len(), runs[1].len()), (2_000, 50));
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].contains("command 0"), "{warnings:?}");
            assert!(warnings[0].contains("dash runs"), "{warnings:?}");
        }
    }

    #[test]
    fn dashed_polylines_within_the_run_budget_still_lower_to_dashes() {
        // 40 crossings of the pane at width 2 is ~4,000 px of dotted path: ~400 runs.
        let (prims, pool, warnings) = decode_in(&format!("[{}]", zigzag_command(40, 2.0, 1)), PANE);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(prims.len() > 100 && prims.len() <= MAX_DASH_RUNS as usize);
        solid_runs(&prims, &pool);
    }

    #[test]
    fn the_run_budget_is_per_command_and_keeps_command_order() {
        let dense = zigzag_command(2_000, 2.0, 1);
        let (prims, pool, warnings) = decode_in(
            &format!(
                r##"[
                    {{"c":"rect","x":0,"y":0,"w":5,"h":5,"color":"#111111"}},
                    {dense},
                    {{"c":"polyline","points":[0,50,200,50],"color":"#000","width":2,"style":2}},
                    {dense},
                    {{"c":"rect","x":9,"y":9,"w":5,"h":5,"color":"#222222"}}
                ]"##
            ),
            PANE,
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("command 1") && warnings[1].contains("command 3"));
        assert!(matches!(prims.first(), Some(Prim::Rect { .. })));
        assert!(matches!(prims.last(), Some(Prim::Rect { .. })));
        let middle = &prims[1..prims.len() - 1];
        assert!(matches!(
            middle[0],
            Prim::Polyline {
                point_count: 2_000,
                ..
            }
        ));
        assert!(matches!(
            middle[middle.len() - 1],
            Prim::Polyline {
                point_count: 2_000,
                ..
            }
        ));
        // The in-budget dashed line between them is lowered to its nine dashes.
        let dashes = solid_runs(&middle[1..middle.len() - 1], &pool);
        assert_eq!(x_spans(&dashes), dash_spans(0.0, 200.0, 12.0, 24.0));
    }

    #[test]
    fn dashed_polyline_work_is_bounded_by_the_budget_for_any_path_shape() {
        // Dense zigzags that stay inside, cross, or mostly leave the pane, plus a spiral of
        // segments that re-enter it: however the path winds, one command is capped.
        let spiral: String = (0..3_000)
            .map(|i| {
                let t = f64::from(i) * 0.05;
                format!("{},{}", 200.0 + 900.0 * t.cos(), 50.0 + 400.0 * t.sin())
            })
            .collect::<Vec<_>>()
            .join(",");
        let spiral = format!(
            r##"{{"c":"polyline","points":[{spiral}],"color":"#000","width":1,"style":1}}"##
        );
        for command in [
            zigzag_command(10_000, 1.0, 1),
            zigzag_command(10_000, 0.5, 1),
            zigzag_command(3_000, 0.0, 2),
            spiral,
        ] {
            let (prims, pool, _) = decode_in(&format!("[{command}]"), PANE);
            assert!(
                prims.len() <= MAX_DASH_RUNS as usize,
                "{} prims from one command",
                prims.len()
            );
            assert!(pool.len() <= 10_000 + 2 * MAX_DASH_RUNS as usize);
        }
    }

    #[test]
    fn malformed_dashed_polylines_leave_the_pool_untouched() {
        let mut pool = vec![[9.0, 9.0]];
        let out = decode_commands(
            r##"[
                {"c":"polyline","points":[2,2, "bad",3],"color":"#000","width":2,"style":2},
                {"c":"polyline","points":[0,0, 1,1, 5],"color":"#000","width":2,"style":2},
                {"c":"polyline","points":[0,0],"color":"#000","width":2,"style":2},
                {"c":"polyline","points":[0,0,50,0],"width":2,"style":2},
                {"c":"polyline","points":[0,0,50,0],"color":"#000","style":2},
                {"c":"polyline","points":[0,0,50,0],"width":2,"style":0},
                {"c":"polyline","points":[0,0,50,0],"color":"#000","style":0}
            ]"##,
            &mut pool,
            &defaults(),
            PANE,
        );
        // Only the trailing odd value is dropped from the second run, leaving one 2-point run.
        assert_eq!(out.prims.len(), 1);
        assert_eq!(out.warnings.len(), 6, "{:?}", out.warnings);
        assert!(matches!(out.prims[0], Prim::Polyline { .. }));
        assert_eq!(
            pool.len(),
            1 + out
                .prims
                .iter()
                .map(|prim| match prim {
                    Prim::Polyline { point_count, .. } => *point_count as usize,
                    _ => 0,
                })
                .sum::<usize>()
        );
        assert_eq!(pool[0], [9.0, 9.0]);
    }

    /// Dash rects an executor would paint for one crisp prim, clipped to `[low, high]`.
    fn crisp_rects(prim: &Prim, low: f32, high: f32) -> Vec<(f32, f32)> {
        let mut instances = Vec::new();
        aeris_charts_render_wgpu::prim_to_instances(prim, &mut instances);
        instances
            .iter()
            .map(|i| match prim {
                Prim::HLine { .. } => (i.rect[0], i.rect[0] + i.rect[2]),
                _ => (i.rect[1], i.rect[1] + i.rect[3]),
            })
            .map(|(a, b)| (a.max(low), b.min(high)))
            .filter(|(a, b)| b > a)
            .collect()
    }

    #[test]
    fn dashed_crisp_lines_reaching_far_past_the_pane_stay_bounded_and_in_phase() {
        let expected_start = |value: i64| (value + 1_000_000_000).rem_euclid(24);
        let command = |extent: &str| {
            format!(
                r##"[{{"c":"hline","y":50,"x1":{extent},"color":"#000","width":2,"style":2}}]"##
            )
        };
        // -1e9..1e9 is a genuine executor hang unclamped: the executors' f32 dash position stops
        // advancing (1e9 + 12 rounds back to 1e9).
        assert_eq!(-1.0e9_f32 + 12.0, -1.0e9_f32);
        let (prims, _, warnings) = decode_in(&command("-1000000000,\"x2\":1000000000"), PANE);
        assert!(warnings.is_empty());
        let [Prim::HLine {
            x0,
            x1,
            width,
            style,
            ..
        }] = prims[..]
        else {
            panic!("{prims:?}");
        };
        assert_eq!((x1, width, style), (400, 2, LineStyle::Dashed));
        assert!((-24..=0).contains(&x0), "{x0}");
        assert_eq!(
            expected_start(i64::from(x0)),
            0,
            "dash phase moves in whole periods"
        );
        let rects = crisp_rects(&prims[0], f32::NEG_INFINITY, f32::INFINITY);
        assert!(rects.len() <= 400 / 24 + 2, "{} rects", rects.len());

        // The f32 stall region: a span that starts past 2^28 misses the pane entirely.
        let (prims, _, warnings) = decode_in(&command("300000000,\"x2\":300001000"), PANE);
        assert!(warnings.is_empty());
        assert!(prims.is_empty());
        let (prims, _, _) = decode_in(&command("-300000000,\"x2\":300000000"), PANE);
        assert_eq!(prims.len(), 1);
        assert!(crisp_rects(&prims[0], f32::NEG_INFINITY, f32::INFINITY).len() <= 400 / 24 + 2);

        // Same ink inside the pane as the unclamped (finite) line.
        let (prims, _, _) = decode_in(&command("-1000,\"x2\":900"), PANE);
        let unclamped = Prim::HLine {
            y: 50,
            x0: -1000,
            x1: 900,
            width: 2,
            style: LineStyle::Dashed,
            color: Color::rgb(0, 0, 0),
        };
        assert_eq!(
            crisp_rects(&prims[0], 0.0, 400.0),
            crisp_rects(&unclamped, 0.0, 400.0)
        );

        // Solid lines are one bounded rect and pass through unchanged.
        let (prims, _, _) = decode_in(
            r##"[{"c":"hline","y":50,"x1":-1000000000,"x2":1000000000,"color":"#000","width":2,"style":0}]"##,
            PANE,
        );
        assert!(matches!(
            prims[..],
            [Prim::HLine {
                x0: -1_000_000_000,
                x1: 1_000_000_000,
                ..
            }]
        ));

        // Empty and reversed dashed extents draw nothing on any executor.
        let (prims, _, _) = decode_in(&command("300,\"x2\":100"), PANE);
        assert!(prims.is_empty());
    }

    #[test]
    fn dashed_vertical_lines_clamp_to_the_pane_like_horizontal_ones() {
        let (prims, _, warnings) = decode_in(
            r##"[
                {"c":"vline","x":30,"y1":-1000000000,"y2":1000000000,"color":"#000","width":2,"style":1},
                {"c":"vline","x":30,"y1":300000000,"y2":300001000,"color":"#000","width":2,"style":1}
            ]"##,
            PANE,
        );
        assert!(warnings.is_empty());
        let [Prim::VLine {
            x, y0, y1, style, ..
        }] = prims[..]
        else {
            panic!("{prims:?}");
        };
        assert_eq!((x, y1, style), (30, 100, LineStyle::Dotted));
        // Dotted at width 2 has a 10 px period.
        assert!((-10..=0).contains(&y0), "{y0}");
        assert_eq!((i64::from(y0) + 1_000_000_000).rem_euclid(10), 0);
        assert!(crisp_rects(&prims[0], f32::NEG_INFINITY, f32::INFINITY).len() <= 100 / 10 + 2);
    }

    #[test]
    fn pane_clip_is_the_absolute_scissor_rect() {
        assert_eq!(
            pane_clip([0, 0, 640, 480]),
            Rect {
                left: 0.0,
                top: 0.0,
                right: 640.0,
                bottom: 480.0
            }
        );
        // A left price axis offsets the pane: the clip stays in the commands' absolute px.
        assert_eq!(
            pane_clip([96, 12, 800, 500]),
            Rect {
                left: 96.0,
                top: 12.0,
                right: 896.0,
                bottom: 512.0
            }
        );
    }
}
