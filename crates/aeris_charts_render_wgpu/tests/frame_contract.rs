//! Shared-frame contract coverage for the browser Canvas2D and WebGPU adapters.
//!
//! This does not require a physical GPU: it proves that one engine fixture can be consumed by
//! both adapter translators, including the marker/round-rect primitives that previously had a
//! silent WebGPU hole.

use aeris_charts_engine::{
    marker_pos, marker_shape, AxisDimension, CategoryScaleType, ChartEngine, ContinuousScaleType,
    GeneralAxisOptions, GeneralScaleType, GeneralSeriesOptions, GeneralXyInput, HorizontalDomain,
    IndicatorInputSource, IndicatorKind, IndicatorOutputStyle, Marker, PriceLine, SeriesKind,
};
use aeris_charts_render::canvas2d::{execute, Canvas2d, Viewport};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, Prim, RasterImage};
use std::sync::Arc;

use aeris_charts_render_wgpu::{
    geom_prims_to_tris, prims_to_group, prims_to_instances, DrawGroup, DrawRun, RunPipeline,
    TexQuadInstance,
};

/// A minimal text prim for scheduling tests (content is irrelevant to the group builder).
fn text_prim(x: f32) -> Prim {
    Prim::Text {
        x,
        y: 0.0,
        text: "txt".into(),
        color: Color::rgb(0, 0, 0),
        size: 12.0,
        family: "Test".into(),
        align: aeris_charts_render::draw_list::TextAlign::Left,
        weight: 400,
        italic: false,
    }
}

/// A resolver that maps every text prim to a dummy 2x2 atlas quad at its anchor.
fn dummy_quad(prim: &Prim) -> Option<TexQuadInstance> {
    let (x, y, transform) = match prim {
        Prim::Text { x, y, .. } => (x, y, [0.0, 0.0, 0.0, 1.0]),
        Prim::RotatedText { x, y, angle, .. } => (x, y, [*x, *y, angle.cos(), angle.sin()]),
        _ => return None,
    };
    Some(TexQuadInstance {
        rect: [*x, *y, 2.0, 2.0],
        uv: [0.0, 0.0, 0.5, 0.5],
        color: transform,
    })
}

#[derive(Default)]
struct CountingCanvas {
    calls: usize,
    fill_color: Option<Color>,
    rects: Vec<([f32; 4], Color)>,
    arcs: usize,
    fills: usize,
}

impl Canvas2d for CountingCanvas {
    fn set_fill_solid(&mut self, color: Color) {
        self.calls += 1;
        self.fill_color = Some(color);
    }
    fn set_fill_vgradient(&mut self, _: f32, _: f32, _: Color, _: Color) {
        self.calls += 1;
    }
    fn set_stroke(&mut self, _: Color) {
        self.calls += 1;
    }
    fn set_line_width(&mut self, _: f32) {
        self.calls += 1;
    }
    fn set_line_dash(&mut self, _: &[f32]) {
        self.calls += 1;
    }
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.calls += 1;
        self.rects.push((
            [x, y, w, h],
            self.fill_color.expect("fill style before rect"),
        ));
    }
    fn fill_rotated_text(
        &mut self,
        _: &str,
        _: f32,
        _: f32,
        _: &str,
        _: Color,
        _: aeris_charts_render::draw_list::TextAlign,
        _: f32,
    ) {
        self.calls += 1;
    }
    fn begin_path(&mut self) {
        self.calls += 1;
    }
    fn move_to(&mut self, _: f32, _: f32) {
        self.calls += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {
        self.calls += 1;
    }
    fn close_path(&mut self) {
        self.calls += 1;
    }
    fn arc(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.calls += 1;
        self.arcs += 1;
    }
    fn stroke(&mut self) {
        self.calls += 1;
    }
    fn fill(&mut self) {
        self.calls += 1;
        self.fills += 1;
    }
}

fn fixture() -> ChartEngine {
    fixture_with_line_style(0)
}

/// The shared engine fixture with `line_style` (the reference numeric style: 0 solid, 1 dotted,
/// 2 dashed) on the main series line and on one indicator output line.
fn fixture_with_line_style(line_style: u8) -> ChartEngine {
    let mut chart = ChartEngine::new(320.0, 220.0, 1.0);
    let times: Vec<f64> = (0..24).map(|i| i as f64).collect();
    let open: Vec<f64> = (0..24).map(|i| 100.0 + i as f64 * 0.2).collect();
    let high: Vec<f64> = open.iter().map(|v| v + 1.2).collect();
    let low: Vec<f64> = open.iter().map(|v| v - 1.0).collect();
    let close: Vec<f64> = open
        .iter()
        .enumerate()
        .map(|(i, v)| v + if i % 2 == 0 { 0.6 } else { -0.4 })
        .collect();
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart.series[0].kind = SeriesKind::Line;
    chart.series[0].line_style = line_style;
    let rsi = chart
        .add_indicator_kind_with_input(
            0,
            IndicatorInputSource::Hlc3,
            IndicatorKind::Rsi {
                period: 3,
                seed: aeris_charts_engine::IndicatorSeed::Sma,
            },
            None,
        )
        .into_iter()
        .next()
        .expect("RSI output");
    let sma = chart.add_sma(rsi, 2).expect("SMA output");
    if line_style != 0 {
        assert!(chart.set_indicator_output_style(
            sma,
            IndicatorOutputStyle {
                visible: true,
                line_style,
                ..IndicatorOutputStyle::default()
            }
        ));
    }
    let bands = chart.add_bollinger(sma, 2, 2.0);
    assert!(chart.set_indicator_output_style(
        bands[0],
        IndicatorOutputStyle {
            area_top_color: Some("rgba(20, 120, 220, 0.24)".into()),
            area_bottom_color: Some("rgba(20, 120, 220, 0.04)".into()),
            ..IndicatorOutputStyle::default()
        }
    ));
    chart.series[0].point_markers = true;
    chart.series[0].last_price_animation = true;
    chart.series[0].markers.push(Marker {
        time: 8,
        position: marker_pos::ABOVE,
        shape: marker_shape::SQUARE,
        color: Color::rgb(0x10, 0x80, 0xff),
        text: "BUY".into(),
        id: String::new(),
        size: 1.0,
        price: None,
    });
    chart.series[0].price_lines.push(PriceLine {
        id: 1,
        price: 102.0,
        color: Color::rgb(0xff, 0x98, 0x00),
        width: 1,
        style: LineStyle::Dashed,
        title: "target".into(),
        line_visible: true,
        axis_label_visible: true,
        axis_label_color: None,
        axis_label_text_color: None,
    });
    let baseline = chart.add_series(SeriesKind::Baseline);
    chart
        .set_series_data(baseline, &times, &open, &high, &low, &close)
        .unwrap();
    let candles = chart.add_series(SeriesKind::Candlestick);
    chart
        .set_series_data(candles, &times, &open, &high, &low, &close)
        .unwrap();
    let bars = chart.add_series(SeriesKind::Bar);
    chart
        .set_series_data(bars, &times, &open, &high, &low, &close)
        .unwrap();
    let histogram = chart.add_series(SeriesKind::Histogram);
    chart
        .set_series_data(histogram, &times, &open, &open, &open, &close)
        .unwrap();
    chart.time_scale.set_width(320.0);
    chart.fit_content();
    chart
}

#[test]
fn one_engine_frame_is_consumable_by_canvas2d_and_webgpu_adapters() {
    let mut chart = fixture();
    let frame = chart.build_frame();
    let mut canvas = CountingCanvas::default();
    let mut quads = Vec::new();
    let mut fill_tris = Vec::new();
    let mut stroke_tris = Vec::new();
    let mut saw_marker_text = false;

    for pane in &frame.panes {
        execute(
            &pane.under,
            &pane.points,
            &mut canvas,
            Viewport {
                width: 320.0,
                height: 220.0,
            },
        );
        execute(
            &pane.main,
            &pane.points,
            &mut canvas,
            Viewport {
                width: 320.0,
                height: 220.0,
            },
        );
        saw_marker_text |= pane
            .under
            .iter()
            .chain(&pane.main)
            .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == "BUY"));
        prims_to_instances(&pane.under, &mut quads);
        prims_to_instances(&pane.main, &mut quads);
        geom_prims_to_tris(&pane.main, &pane.points, &mut fill_tris, &mut stroke_tris);
    }

    assert!(
        canvas.calls > 0,
        "Canvas2D adapter must execute the shared frame"
    );
    assert!(
        !quads.is_empty(),
        "WebGPU quad adapter must receive rect/grid primitives"
    );
    assert!(
        !fill_tris.is_empty(),
        "WebGPU triangle adapter must receive area primitives"
    );
    assert!(
        !stroke_tris.is_empty(),
        "WebGPU triangle adapter must receive line/marker primitives"
    );
    assert!(
        saw_marker_text,
        "the official marker label must remain in the shared frame"
    );

    // Integer geometry is intentionally backend-identical: both adapters use the same rect and
    // dash expansion rules. Compare the complete command stream, not just that each backend ran.
    assert_eq!(
        canvas.rects.len(),
        quads.len(),
        "Canvas2D/WebGPU rect count diverged"
    );
    for ((canvas_rect, canvas_color), gpu) in canvas.rects.iter().zip(&quads) {
        assert_eq!(
            canvas_rect, &gpu.rect,
            "Canvas2D/WebGPU rect geometry diverged"
        );
        let expected = [
            canvas_color.r() as f32 / 255.0,
            canvas_color.g() as f32 / 255.0,
            canvas_color.b() as f32 / 255.0,
            canvas_color.a() as f32 / 255.0,
        ];
        assert_eq!(gpu.color, expected, "Canvas2D/WebGPU rect color diverged");
    }
}

/// The WebGPU stroker ignores `Prim::Polyline::style` (it has no dash concept), so the frame
/// producer must lower every dashed or dotted line to solid dash runs before a frame reaches it.
/// Pins that producer contract at the WebGPU boundary for a dashed series line and a dashed
/// indicator output, in every layer a backend executes.
#[test]
fn engine_frames_reach_the_webgpu_stroker_with_only_solid_polylines() {
    let polylines = |line_style: u8| {
        let frame = fixture_with_line_style(line_style).build_frame();
        let mut count = 0;
        for pane in &frame.panes {
            let layers = [&pane.under, &pane.main, &pane.top_prims];
            for prim in layers.into_iter().flatten() {
                if let Prim::Polyline { style, .. } = prim {
                    assert_eq!(
                        *style,
                        LineStyle::Solid,
                        "line style {line_style} reached the WebGPU boundary dashed: {prim:?}"
                    );
                    count += 1;
                }
            }
        }
        count
    };
    let solid = polylines(0);
    // Vacuousness guard: dashing really happened, as extra runs of the same strokes.
    assert!(polylines(2) > solid, "dashed lines lower to dash runs");
    assert!(polylines(1) > solid, "dotted lines lower to dash runs");
}

#[test]
fn category_column_segment_is_identical_for_canvas2d_and_webgpu_quads() {
    let mut chart = ChartEngine::new(320.0, 220.0, 1.0);
    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Category {
                scale: CategoryScaleType::Band,
            },
        )
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Band,
        ))
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    let dataset = chart
        .create_general_xy_dataset(GeneralXyInput::Category {
            ids: None,
            categories: vec!["A".into(), "B".into(), "C".into()],
            category_indices: vec![0, 1, 2],
            y: vec![-2.0, 5.0, 99.0],
            y_valid: Some(vec![1, 1, 0]),
        })
        .unwrap();
    let mut options = GeneralSeriesOptions::column(pane, dataset, "x", "y");
    options.color = Some("#345678".into());
    chart.add_general_series(options).unwrap();
    chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

    let frame = chart.build_frame();
    let pane_frame = &frame.panes[pane];
    let expected = Color::parse_css("#345678").unwrap();
    let segment = chart
        .frame_series_segments(pane)
        .iter()
        .find(|segment| {
            pane_frame.main
                [segment.start.min(pane_frame.main.len())..segment.end.min(pane_frame.main.len())]
                .iter()
                .any(
                    |primitive| matches!(primitive, Prim::Rect { color, .. } if *color == expected),
                )
        })
        .expect("general series segment must reach retained backends");
    let prims = &pane_frame.main[segment.start..segment.end];

    let mut canvas = CountingCanvas::default();
    execute(
        prims,
        &pane_frame.points,
        &mut canvas,
        Viewport {
            width: frame.width as f32,
            height: frame.height as f32,
        },
    );
    let mut quads = Vec::new();
    prims_to_instances(prims, &mut quads);
    assert_eq!(canvas.rects.len(), 2);
    assert_eq!(quads.len(), 2);
    for ((canvas_rect, canvas_color), gpu) in canvas.rects.iter().zip(&quads) {
        assert_eq!(canvas_rect, &gpu.rect);
        assert_eq!(*canvas_color, expected);
        assert_eq!(
            gpu.color,
            [
                expected.r() as f32 / 255.0,
                expected.g() as f32 / 255.0,
                expected.b() as f32 / 255.0,
                expected.a() as f32 / 255.0,
            ]
        );
    }
}

#[test]
fn xy_scatter_segment_reaches_canvas2d_and_webgpu_circle_paths() {
    let mut chart = ChartEngine::new(360.0, 240.0, 1.0);
    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    let dataset = chart
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: vec![1.0, 2.0, 3.0, 4.0],
            y: vec![-2.0, 1.0, 5.0, 99.0],
            y_valid: Some(vec![1, 1, 1, 0]),
        })
        .unwrap();
    let mut options = GeneralSeriesOptions::scatter(pane, dataset, "x", "y");
    options.color = Some("#267f99".into());
    chart.add_general_series(options).unwrap();
    chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

    let frame = chart.build_frame();
    let pane_frame = &frame.panes[pane];
    let expected = Color::parse_css("#267f99").unwrap();
    let segment = chart
        .frame_series_segments(pane)
        .iter()
        .find(|segment| {
            pane_frame.main
                [segment.start.min(pane_frame.main.len())..segment.end.min(pane_frame.main.len())]
                .iter()
                .any(
                    |primitive| matches!(primitive, Prim::Circle { fill, .. } if *fill == expected),
                )
        })
        .expect("scatter segment must reach retained backends");
    let prims = &pane_frame.main[segment.start..segment.end];
    assert_eq!(
        prims
            .iter()
            .filter(|primitive| matches!(primitive, Prim::Circle { fill, .. } if *fill == expected))
            .count(),
        3
    );

    let mut canvas = CountingCanvas::default();
    execute(
        prims,
        &pane_frame.points,
        &mut canvas,
        Viewport {
            width: frame.width as f32,
            height: frame.height as f32,
        },
    );
    assert_eq!(canvas.arcs, 3);
    assert_eq!(canvas.fills, 3);

    let mut fill_tris = Vec::new();
    let mut stroke_tris = Vec::new();
    geom_prims_to_tris(prims, &pane_frame.points, &mut fill_tris, &mut stroke_tris);
    assert!(
        fill_tris.is_empty(),
        "filled discs use the ordered geometry bucket, not the area-fill bucket"
    );
    assert!(
        !stroke_tris.is_empty(),
        "WebGPU must tessellate scatter circles into ordered geometry"
    );
}

#[test]
fn xy_line_segment_reaches_canvas2d_and_webgpu_stroke_paths() {
    let mut chart = ChartEngine::new(360.0, 240.0, 1.0);
    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    let dataset = chart
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: vec![0.0, 1.0, 2.0, 3.0, 4.0],
            y: vec![0.0, 1.0, 99.0, 3.0, 4.0],
            y_valid: Some(vec![1, 1, 0, 1, 1]),
        })
        .unwrap();
    let mut options = GeneralSeriesOptions::xy_line(pane, dataset, "x", "y");
    options.color = Some("#2d6b8f".into());
    chart.add_general_series(options).unwrap();
    chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

    let frame = chart.build_frame();
    let pane_frame = &frame.panes[pane];
    let expected = Color::parse_css("#2d6b8f").unwrap();
    let segment = chart
        .frame_series_segments(pane)
        .iter()
        .find(|segment| {
            pane_frame.main
                [segment.start.min(pane_frame.main.len())..segment.end.min(pane_frame.main.len())]
                .iter()
                .any(|primitive| {
                    matches!(primitive, Prim::Polyline { color, .. } if *color == expected)
                })
        })
        .expect("XY line segment must reach retained backends");
    let prims = &pane_frame.main[segment.start..segment.end];
    assert_eq!(
        prims
            .iter()
            .filter(|primitive| matches!(primitive, Prim::Polyline { color, point_count: 2, .. } if *color == expected))
            .count(),
        2,
        "missing Y must split XY line into two stroke runs"
    );

    let mut canvas = CountingCanvas::default();
    execute(
        prims,
        &pane_frame.points,
        &mut canvas,
        Viewport {
            width: frame.width as f32,
            height: frame.height as f32,
        },
    );
    let canvas_calls = canvas.calls;

    let mut fill_tris = Vec::new();
    let mut stroke_tris = Vec::new();
    geom_prims_to_tris(prims, &pane_frame.points, &mut fill_tris, &mut stroke_tris);
    assert!(
        canvas_calls > 0,
        "Canvas2D must execute the line primitives"
    );
    assert!(
        fill_tris.is_empty(),
        "XY lines must not create fill geometry"
    );
    assert!(
        !stroke_tris.is_empty(),
        "WebGPU must tessellate XY line polylines into stroke geometry"
    );
}

#[test]
fn xy_area_segment_reaches_canvas2d_and_webgpu_fill_and_stroke_paths() {
    let mut chart = ChartEngine::new(360.0, 240.0, 1.0);
    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    let dataset = chart
        .create_general_xy_dataset(GeneralXyInput::Numeric {
            ids: None,
            x: vec![0.0, 1.0, 2.0, 3.0, 4.0],
            y: vec![1.0, 2.0, 99.0, 3.0, 1.0],
            y_valid: Some(vec![1, 1, 0, 1, 1]),
        })
        .unwrap();
    chart
        .add_general_series(GeneralSeriesOptions::xy_area(pane, dataset, "x", "y"))
        .unwrap();
    chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

    let frame = chart.build_frame();
    let pane_frame = &frame.panes[pane];
    assert_eq!(
        pane_frame
            .main
            .iter()
            .filter(|primitive| matches!(primitive, Prim::AreaFill { point_count: 2, .. }))
            .count(),
        2
    );
    let mut canvas = CountingCanvas::default();
    execute(
        &pane_frame.main,
        &pane_frame.points,
        &mut canvas,
        Viewport {
            width: frame.width as f32,
            height: frame.height as f32,
        },
    );
    let mut fill_tris = Vec::new();
    let mut stroke_tris = Vec::new();
    geom_prims_to_tris(
        &pane_frame.main,
        &pane_frame.points,
        &mut fill_tris,
        &mut stroke_tris,
    );
    assert!(canvas.calls > 0);
    assert!(
        !fill_tris.is_empty(),
        "WebGPU must tessellate XY area fills"
    );
    assert!(
        !stroke_tris.is_empty(),
        "WebGPU must tessellate XY area strokes"
    );
}

#[test]
fn range_area_segment_reaches_canvas2d_and_webgpu_fill_and_stroke_paths() {
    let mut chart = ChartEngine::new(360.0, 240.0, 1.0);
    let pane = chart
        .add_pane_with_domain(
            true,
            HorizontalDomain::Continuous {
                scale: ContinuousScaleType::Linear,
            },
        )
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "x",
            pane,
            AxisDimension::X,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    chart
        .add_general_axis(GeneralAxisOptions::new(
            "y",
            pane,
            AxisDimension::Y,
            GeneralScaleType::Linear,
        ))
        .unwrap();
    let dataset = chart
        .create_general_xy_dataset(GeneralXyInput::RangeNumeric {
            ids: None,
            x: vec![0.0, 1.0, 2.0, 3.0, 4.0],
            low: vec![1.0, 2.0, 0.0, 3.0, 1.0],
            low_valid: Some(vec![1, 1, 0, 1, 1]),
            high: vec![4.0, 5.0, 6.0, 7.0, 5.0],
            high_valid: None,
        })
        .unwrap();
    chart
        .add_general_series(GeneralSeriesOptions::range_area(pane, dataset, "x", "y"))
        .unwrap();
    chart.recompute_layout_with_measure(true, |text, _| text.len() as f64 * 7.0, |_, _| 0.0);

    let frame = chart.build_frame();
    let pane_frame = &frame.panes[pane];
    assert_eq!(
        pane_frame
            .main
            .iter()
            .filter(|primitive| matches!(primitive, Prim::BandFill { point_count: 2, .. }))
            .count(),
        2,
        "missing bounds must split the range band into two fill runs"
    );
    assert_eq!(
        pane_frame
            .main
            .iter()
            .filter(|primitive| matches!(primitive, Prim::Polyline { point_count: 2, .. }))
            .count(),
        4,
        "each range run must retain both boundary strokes"
    );

    let mut canvas = CountingCanvas::default();
    execute(
        &pane_frame.main,
        &pane_frame.points,
        &mut canvas,
        Viewport {
            width: frame.width as f32,
            height: frame.height as f32,
        },
    );
    let mut fill_tris = Vec::new();
    let mut stroke_tris = Vec::new();
    geom_prims_to_tris(
        &pane_frame.main,
        &pane_frame.points,
        &mut fill_tris,
        &mut stroke_tris,
    );
    assert!(
        canvas.calls > 0,
        "Canvas2D must execute the range primitives"
    );
    assert!(
        !fill_tris.is_empty(),
        "WebGPU must tessellate range-area band fills"
    );
    assert!(
        !stroke_tris.is_empty(),
        "WebGPU must tessellate range-area boundary strokes"
    );
}

/// Runs must tile their pipeline's buffer contiguously, in ascending order, covering it exactly.
fn assert_runs_tile_buffers(group: &DrawGroup) {
    for (pipeline, len) in [
        (RunPipeline::Tri, group.tris.len()),
        (RunPipeline::Quad, group.quads.len()),
        (RunPipeline::ImageQuad, group.image_quads.len()),
    ] {
        let mut next = 0u32;
        for run in group.runs.iter().filter(|r| r.pipeline == pipeline) {
            assert_eq!(run.first, next, "{pipeline:?} runs must be contiguous");
            assert!(run.count > 0, "empty runs are never recorded");
            next += run.count;
        }
        assert_eq!(
            next as usize, len,
            "{pipeline:?} runs must cover the buffer"
        );
    }
    let mut next = 0u32;
    for run in group.runs.iter().filter(|run| {
        matches!(
            run.pipeline,
            RunPipeline::TexQuad | RunPipeline::RotatedTexQuad
        )
    }) {
        assert_eq!(
            run.first, next,
            "text runs must share one contiguous buffer"
        );
        next += run.count;
    }
    assert_eq!(next, group.tex_quads.len() as u32);
}

#[test]
fn group_builder_preserves_mixed_prim_order_with_run_length_batching() {
    use aeris_charts_render::draw_list::{IRect, LineType};
    // The Canvas2D executor paints strictly in prim order; the WebGPU schedule must encode
    // the same observable order, one draw call per maximal same-pipeline run.
    let rect = |x| Prim::Rect {
        rect: IRect {
            x,
            y: 0,
            w: 4,
            h: 4,
        },
        color: Color::rgb(0, 0, 0),
    };
    let circle = |x| Prim::Circle {
        cx: x,
        cy: 5.0,
        radius: 3.0,
        fill: Color::rgb(0xFF, 0, 0),
        stroke_width: 0.0,
        stroke: Color::rgb(0, 0, 0),
    };
    let points = [[0.0f32, 0.0], [10.0, 5.0], [20.0, 0.0]];
    let prims = [
        rect(0),
        circle(8.0),
        rect(20),
        rect(30), // batches with the previous quad
        Prim::Polyline {
            first_point: 0,
            point_count: 3,
            width: 2.0,
            style: LineStyle::Solid,
            line_type: LineType::Simple,
            color: Color::rgb(0, 0, 0xFF),
        },
        circle(40.0),   // batches with the previous tri run
        text_prim(0.0), // unresolved (None) text reserves no slot and splits no run
        rect(50),
        // Degenerate geometry emits nothing and must not split the run either.
        Prim::Rect {
            rect: IRect {
                x: 0,
                y: 0,
                w: 0,
                h: 5,
            },
            color: Color::rgb(0, 0, 0),
        },
        rect(60), // still the same quad run as rect(50)
    ];
    let mut group = DrawGroup::default();
    prims_to_group(&prims, &points, &mut group, &mut |_| None, &mut |_| None);

    let schedule: Vec<RunPipeline> = group.runs.iter().map(|r| r.pipeline).collect();
    assert_eq!(
        schedule,
        [
            RunPipeline::Quad,
            RunPipeline::Tri,
            RunPipeline::Quad,
            RunPipeline::Tri,
            RunPipeline::Quad,
        ],
        "the schedule must reproduce the Canvas2D family order exactly"
    );
    assert_eq!(group.quads.len(), 5);
    assert_eq!(
        group.runs[0],
        DrawRun {
            pipeline: RunPipeline::Quad,
            first: 0,
            count: 1
        }
    );
    assert_eq!(
        group.runs[2],
        DrawRun {
            pipeline: RunPipeline::Quad,
            first: 1,
            count: 2
        },
        "consecutive quads must batch into one run"
    );
    assert_eq!(
        group.runs[4],
        DrawRun {
            pipeline: RunPipeline::Quad,
            first: 3,
            count: 2
        },
        "degenerate prims and text slots must not split a run"
    );
    let tri_runs: Vec<_> = group
        .runs
        .iter()
        .filter(|r| r.pipeline == RunPipeline::Tri)
        .collect();
    assert_eq!(tri_runs.len(), 2);
    assert_eq!(tri_runs[0].first, 0);
    assert_eq!(
        tri_runs[1].first,
        tri_runs[0].first + tri_runs[0].count,
        "tri runs must be adjacent ranges of one buffer"
    );
    assert_runs_tile_buffers(&group);
}

#[test]
fn group_builder_schedules_resolved_text_quads_in_prim_order() {
    use aeris_charts_render::draw_list::IRect;
    let rect = |x| Prim::Rect {
        rect: IRect {
            x,
            y: 0,
            w: 4,
            h: 4,
        },
        color: Color::rgb(0, 0, 0),
    };
    // Two text runs around one rect: the tex quads must land between the quad runs, batching
    // with each other, mirroring the Canvas2D paint order rect → text → text → rect.
    let prims = [rect(0), text_prim(10.0), text_prim(20.0), rect(30)];
    let mut group = DrawGroup::default();
    prims_to_group(&prims, &[], &mut group, &mut dummy_quad, &mut |_| None);

    let schedule: Vec<RunPipeline> = group.runs.iter().map(|r| r.pipeline).collect();
    assert_eq!(
        schedule,
        [RunPipeline::Quad, RunPipeline::TexQuad, RunPipeline::Quad],
        "text quads must schedule at their prim position, not above everything"
    );
    assert_eq!(group.tex_quads.len(), 2);
    assert_eq!(
        group.runs[1],
        DrawRun {
            pipeline: RunPipeline::TexQuad,
            first: 0,
            count: 2
        },
        "consecutive text runs batch into one tex-quad draw"
    );
    assert_runs_tile_buffers(&group);
}

#[test]
fn rotated_text_quad_carries_the_canonical_anchor_and_rotation() {
    let angle = -0.75_f32;
    let prim = Prim::RotatedText {
        x: 41.0,
        y: 27.0,
        text: "trend".into(),
        color: Color::rgb(1, 2, 3),
        size: 12.0,
        family: "Test".into(),
        align: aeris_charts_render::draw_list::TextAlign::Center,
        weight: 400,
        italic: false,
        angle,
    };
    let mut group = DrawGroup::default();
    prims_to_group(&[prim], &[], &mut group, &mut dummy_quad, &mut |_| None);
    assert_eq!(group.tex_quads.len(), 1);
    assert_eq!(group.runs[0].pipeline, RunPipeline::RotatedTexQuad);
    assert_eq!(
        group.tex_quads[0].color,
        [41.0, 27.0, angle.cos(), angle.sin()]
    );
}

#[test]
fn group_builder_keeps_images_on_their_dedicated_atlas_pipeline() {
    let prims = [Prim::Image {
        image: RasterImage {
            key: 7,
            width: 1,
            height: 1,
            pixels: Arc::<[u8]>::from([255, 255, 255, 255]),
        },
        rect: [4.0, 5.0, 20.0, 10.0],
        opacity: 0.5,
    }];
    let mut group = DrawGroup::default();
    prims_to_group(&prims, &[], &mut group, &mut |_| None, &mut |_| {
        Some(TexQuadInstance {
            rect: [4.0, 5.0, 20.0, 10.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
        })
    });
    assert!(group.tex_quads.is_empty());
    assert_eq!(group.image_quads.len(), 1);
    assert_eq!(group.runs[0].pipeline, RunPipeline::ImageQuad);
    assert_runs_tile_buffers(&group);
}

#[test]
fn group_builder_batches_candle_blocks_and_schedules_markers_after_candles() {
    let mut chart = fixture();
    let frame = chart.build_frame();
    let mut saw_tri_over_quad = false;
    for pane in &frame.panes {
        let mut group = DrawGroup::default();
        prims_to_group(
            &pane.under,
            &pane.points,
            &mut group,
            &mut |_| None,
            &mut |_| None,
        );
        prims_to_group(
            &pane.main,
            &pane.points,
            &mut group,
            &mut |_| None,
            &mut |_| None,
        );
        prims_to_group(
            &pane.top_prims,
            &pane.points,
            &mut group,
            &mut |_| None,
            &mut |_| None,
        );
        assert_runs_tile_buffers(&group);

        // Perf contract: the candle/bar/histogram blocks (hundreds of quads) stay a handful
        // of draw calls — run-length batching must not degrade into per-prim draws.
        assert!(
            group.quads.len() > 100,
            "fixture should exercise a realistic quad count"
        );
        let quad_runs = group
            .runs
            .iter()
            .filter(|r| r.pipeline == RunPipeline::Quad)
            .count();
        assert!(
            quad_runs <= 6,
            "quad content collapsed to {quad_runs} runs; batching regressed"
        );
        assert!(
            group.runs.len() <= 14,
            "{} draw calls for a candle frame; run-length batching regressed",
            group.runs.len()
        );

        // The ordering regression this builder fixes: engine markers are tri-family prims
        // emitted after the quad-family candles, so the schedule must contain a quad run
        // followed by a tri run (previously every tri bucket painted before every quad).
        saw_tri_over_quad |= group
            .runs
            .windows(2)
            .any(|w| w[0].pipeline == RunPipeline::Quad && w[1].pipeline == RunPipeline::Tri);
    }
    assert!(
        saw_tri_over_quad,
        "the marker fixture must exercise tri-after-quad scheduling"
    );
}
