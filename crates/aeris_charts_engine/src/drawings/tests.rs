//! Headless tests for the engine-owned drawing objects (drawings.rs): model validation,
//! options JSON, hit-testing, anchor/body drag math, and the interactive creation flow.

use aeris_charts_core::style::DEFAULT_PRIMARY_RGB;
use aeris_charts_render::draw_list::{LineType, Prim};

use super::*;

/// One candle series over 10 hourly bars with a settled layout (dpr 1, pane = the full
/// 800×500 content area) — the same fixture as the series hit-test tests.
fn settled_chart() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
    let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

fn x_at(chart: &ChartEngine, logical: f64) -> f64 {
    chart.logical_to_coordinate(logical).unwrap()
}

fn y_at(chart: &ChartEngine, price: f64) -> f64 {
    chart.series_price_to_coordinate(0, price).unwrap()
}

fn add_trend(chart: &mut ChartEngine) -> DrawingId {
    chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.5,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 12.5,
                },
            ],
            None,
        )
        .unwrap()
}

#[test]
fn trend_labels_default_to_top_right_without_changing_standalone_text_defaults() {
    let mut chart = settled_chart();
    let trend = add_trend(&mut chart);
    let trend = chart.drawing(trend).unwrap();
    assert_eq!(trend.text_h_align, DrawingTextHAlign::Right);
    assert_eq!(trend.text_v_align, DrawingTextVAlign::Top);

    let text = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 4.0,
                price: 11.0,
            }],
            None,
        )
        .unwrap();
    let text = chart.drawing(text).unwrap();
    assert_eq!(text.text_h_align, DrawingTextHAlign::Center);
    assert_eq!(text.text_v_align, DrawingTextVAlign::Middle);
}

#[test]
fn named_templates_carry_style_but_never_identity_placement_or_text() {
    let mut chart = settled_chart();
    let points = |from: f64| {
        vec![
            DrawingPoint {
                logical: from,
                price: 10.5,
            },
            DrawingPoint {
                logical: from + 3.0,
                price: 12.5,
            },
        ]
    };
    let source = chart
        .add_drawing(
            DrawingKind::FibRetracement,
            0,
            points(1.0),
            Some(
                r##"{"name":"Swing A","group_id":"g1","text":"A","locked":true,"z_order":7,
                "visible":false,"price_scale_id":"left",
                "interval_visibility":{"enabled":true,"intervals":[]},
                "color":"#123456","width":3,"style":"dotted","text_size":17}"##,
            ),
        )
        .unwrap();
    let target = chart
        .add_drawing(
            DrawingKind::FibRetracement,
            0,
            points(5.0),
            Some(r#"{"name":"Swing B","text":"B"}"#),
        )
        .unwrap();
    // Several edits put the target's revision ahead of the source's.
    for color in ["#654321", "#654322", "#654323"] {
        assert!(chart.drawing_apply_options(target, &format!(r#"{{"color":"{color}"}}"#)));
    }
    assert!(chart.drawing(target).unwrap().revision > chart.drawing(source).unwrap().revision);
    let before = chart.drawing(target).unwrap().clone();

    let template = chart.drawing_template_json(source, "my style").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&template).unwrap();
    for key in TEMPLATE_NON_STYLE_KEYS {
        assert!(parsed["options"].get(*key).is_none(), "{key} in {template}");
    }
    assert!(chart.apply_drawing_template_json(target, &template));
    let after = chart.drawing(target).unwrap();
    // Style transfers.
    assert_eq!(after.color, "#123456");
    assert_eq!(after.width, 3.0);
    assert_eq!(after.style, LineStyle::Dotted);
    assert_eq!(after.text_size, Some(17.0));
    // Identity, placement, visibility, and content stay the target's own.
    assert_eq!(after.name, before.name);
    assert_eq!(after.group_id, before.group_id);
    assert_eq!(after.locked, before.locked);
    assert_eq!(after.visible, before.visible);
    assert_eq!(after.z_order, before.z_order);
    assert_eq!(after.interval_visibility, before.interval_visibility);
    assert_eq!(after.price_scale, before.price_scale);
    assert_eq!(after.text, "B");
    assert!(
        after.revision > before.revision,
        "the revision only moves forward"
    );

    // A host-written template carrying identity keys in either spelling applies style only.
    let written = serde_json::json!({
        "name": "host",
        "kind": "fib_retracement",
        "options": {
            "name": "Stolen", "groupId": "g9", "zOrder": 99, "intervalVisibility": {"enabled": true},
            "priceScaleId": "left", "revision": 1, "locked": true, "text": "Stolen",
            "color": "#abcdef"
        }
    });
    let revision = chart.drawing(target).unwrap().revision;
    assert!(chart.apply_drawing_template_json(target, &written.to_string()));
    let after = chart.drawing(target).unwrap();
    assert_eq!(after.color, "#abcdef");
    assert_eq!(
        (
            after.name.as_str(),
            after.group_id.as_deref(),
            after.z_order,
            after.locked
        ),
        ("Swing B", None, before.z_order, false)
    );
    assert_eq!(after.text, "B");
    assert!(after.revision > revision);
}

#[test]
fn clipboard_payloads_are_bounded_like_persisted_drawings_not_templates() {
    let mut chart = settled_chart();
    let stroke = |count: usize, from: f64| {
        (0..count)
            .map(|index| DrawingPoint {
                logical: from + index as f64 * 0.003_7,
                price: 10.0 + (index as f64 * 0.017).sin(),
            })
            .collect::<Vec<_>>()
    };
    // A long freehand stroke with fractional anchors, well past the 64 KiB template bound.
    let highlighter = chart
        .add_drawing(DrawingKind::Highlighter, 0, stroke(2_000, 1.0), None)
        .unwrap();
    let payload = chart.copy_drawings_json(&[highlighter]).unwrap();
    assert!(payload.len() > crate::MAX_DRAWING_TEMPLATE_BYTES);
    let pasted = chart.paste_drawings_json(&payload, 0, 0.5, 0.0).unwrap();
    assert_eq!(chart.drawing(pasted[0]).unwrap().points.len(), 2_000);
    let clone = chart.clone_drawing(highlighter, 1.0, 0.0).unwrap();
    assert_eq!(chart.drawing(clone).unwrap().points.len(), 2_000);

    // Modest multi-selections: 100 trend lines and 40 retracements (level lists included).
    let mut ids = Vec::new();
    for index in 0..100 {
        ids.push(add_trend(&mut chart));
        assert!(chart.drawing_apply_options(ids[index], &format!(r#"{{"name":"t{index}"}}"#)));
    }
    for _ in 0..40 {
        ids.push(
            chart
                .add_drawing(
                    DrawingKind::FibRetracement,
                    0,
                    vec![
                        DrawingPoint {
                            logical: 2.0,
                            price: 10.5,
                        },
                        DrawingPoint {
                            logical: 6.0,
                            price: 12.5,
                        },
                    ],
                    None,
                )
                .unwrap(),
        );
    }
    let before = chart.drawings().len();
    let payload = chart.copy_drawings_json(&ids).unwrap();
    assert_eq!(
        chart
            .paste_drawings_json(&payload, 0, 0.0, 0.0)
            .unwrap()
            .len(),
        140
    );
    assert_eq!(chart.drawings().len(), before + 140);

    // Nothing to copy is invalid data; past the anchor bound is a resource limit.
    assert_eq!(
        chart.copy_drawings_json(&[9_999]).unwrap_err().code(),
        ErrorCode::InvalidData
    );
    let giants = (0..3)
        .map(|index| {
            chart
                .add_drawing(
                    DrawingKind::Highlighter,
                    0,
                    stroke(MAX_DRAWING_POINTS, index as f64),
                    None,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        chart.copy_drawings_json(&giants).unwrap_err().code(),
        ErrorCode::ResourceLimit
    );
    // A clone of a drawing at the per-drawing bound still works.
    assert!(chart.clone_drawing(giants[0], 0.0, 0.0).is_some());

    // Pasted payloads past either bound change nothing.
    let minimal = |count: usize| {
        let point = r#"{"logical":1,"price":1}"#;
        let points = vec![point; count].join(",");
        format!(r#"{{"kind":"highlighter","pane_index":0,"points":[{points}],"options":{{}}}}"#)
    };
    let payload = |items: usize, per_item: usize| {
        format!(
            r#"{{"schema":"aeris_charts-drawings","revision":1,"drawings":[{}]}}"#,
            vec![minimal(per_item); items].join(",")
        )
    };
    assert_eq!(
        chart
            .paste_drawings_json(&payload(3, 10), 0, 0.0, 0.0)
            .unwrap()
            .len(),
        3,
        "the same shape within the bounds pastes"
    );
    let count = chart.drawings().len();
    let too_many_points = payload(3, crate::MAX_DRAWING_CLIPBOARD_POINTS / 3 + 1);
    assert!(too_many_points.len() <= crate::MAX_DRAWING_CLIPBOARD_BYTES);
    assert!(chart
        .paste_drawings_json(&too_many_points, 0, 0.0, 0.0)
        .is_none());
    let too_many_bytes = format!(
        r#"{{"schema":"aeris_charts-drawings","revision":1,"drawings":[],"pad":"{}"}}"#,
        "x".repeat(crate::MAX_DRAWING_CLIPBOARD_BYTES)
    );
    assert!(chart
        .paste_drawings_json(&too_many_bytes, 0, 0.0, 0.0)
        .is_none());
    assert_eq!(chart.drawings().len(), count);
}

#[test]
fn drawing_history_reverses_create_delete_points_and_style() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let initial_drawing = chart.drawing(id).unwrap().clone();

    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id), Some(&initial_drawing));

    assert!(chart.drawing_apply_options(id, r##"{"color":"#ff0000","width":5}"##));
    let styled = chart.drawing(id).unwrap().clone();
    assert_ne!(styled, initial_drawing);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id), Some(&initial_drawing));
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id), Some(&styled));

    let moved_points = vec![
        DrawingPoint {
            logical: 3.0,
            price: 11.0,
        },
        DrawingPoint {
            logical: 8.0,
            price: 13.0,
        },
    ];
    assert!(chart.drawing_set_points(id, &serde_json::to_string(&moved_points).unwrap()));
    assert_eq!(chart.drawing(id).unwrap().points, moved_points);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id), Some(&styled));
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, moved_points);

    assert!(chart.remove_drawing(id));
    assert!(chart.drawing(id).is_none());
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, moved_points);
    assert!(chart.redo_drawing());
    assert!(chart.drawing(id).is_none());
}

#[test]
fn historical_insert_rebases_fractional_anchors_and_history_without_a_history_command() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [10.0, 20.0, 30.0];
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 0.5,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 1.0,
                    price: 11.0,
                },
            ],
            None,
        )
        .unwrap();

    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
    assert!(chart.update_series_bar(0, 15.0, [10.5; 4]));
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points[0].logical, 1.0);
    assert_eq!(chart.drawing(id).unwrap().points[1].logical, 2.0);
}

#[test]
fn divergent_series_keep_exact_shared_timestamp_anchors() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let divergent = chart.add_series(SeriesKind::Line);
    let primary_times = [10.0, 20.0, 30.0, 40.0];
    let primary_values = [10.0, 11.0, 12.0, 13.0];
    chart
        .set_series_data(
            0,
            &primary_times,
            &primary_values,
            &primary_values,
            &primary_values,
            &primary_values,
        )
        .unwrap();
    chart
        .set_series_data(divergent, &[20.0], &[20.0], &[20.0], &[20.0], &[20.0])
        .unwrap();
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 1.0,
                    price: 11.0,
                },
                DrawingPoint {
                    logical: 3.0,
                    price: 13.0,
                },
            ],
            None,
        )
        .unwrap();

    let replacement_times = [15.0, 30.0, 35.0, 40.0];
    let replacement_values = [10.5, 12.0, 12.5, 13.0];
    chart
        .set_series_data(
            0,
            &replacement_times,
            &replacement_values,
            &replacement_values,
            &replacement_values,
            &replacement_values,
        )
        .unwrap();

    assert_eq!(chart.data.merged_times(), &[15, 20, 30, 35, 40]);
    assert_eq!(chart.drawing(id).unwrap().points[0].logical, 1.0);
    assert_eq!(chart.drawing(id).unwrap().points[1].logical, 4.0);
}

#[test]
fn current_replacement_and_tail_append_do_not_move_drawings() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let points = chart.drawing(id).unwrap().points.clone();

    assert!(chart.update_series_bar(0, 9.0 * 3_600.0, [10.25; 4]));
    assert_eq!(chart.drawing(id).unwrap().points, points);
    assert!(chart.update_series_bar(0, 10.0 * 3_600.0, [10.5; 4]));
    assert_eq!(chart.drawing(id).unwrap().points, points);
}

#[test]
fn cap_trim_rebases_4096_point_drawing_by_the_exact_129_removed_rows() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..4_096).map(|index| index as f64).collect::<Vec<_>>();
    let values = vec![10.0; times.len()];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    assert!(chart.set_series_max_points(0, Some(4_096)));
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 128.5,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 4_095.0,
                    price: 10.0,
                },
            ],
            None,
        )
        .unwrap();

    assert!(chart.update_series_bar(0, 4_096.0, [10.0; 4]));

    assert_eq!(chart.data.merged_times().len(), 3_968);
    assert_eq!(chart.data.merged_times().first(), Some(&129));
    assert_eq!(chart.drawing(id).unwrap().points[0].logical, -0.5);
    assert_eq!(chart.drawing(id).unwrap().points[1].logical, 3_966.0);
}

#[test]
fn active_drawing_state_and_pixel_baselines_rebase_with_the_union() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [10.0, 20.0, 30.0];
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 0.5,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 1.5,
                    price: 11.0,
                },
            ],
            None,
        )
        .unwrap();
    let drawing = chart.drawing(id).unwrap().clone();
    chart.drawing_drag = Some(DrawingDrag {
        id,
        part: DrawingDragPart::Body,
        start_x: 100.0,
        start_y: 100.0,
        current_x: 100.0,
        current_y: 100.0,
        handle_px: (100.0, 100.0),
        history_points: drawing.points.clone(),
        history_tool_options: drawing.tool_options.clone(),
        start_points: drawing.points.clone(),
        start_px: vec![(f64::NAN, f64::NAN); 2],
        keyboard_step: None,
    });
    chart.drawing_controller.pending = Some(PendingDrawing {
        drawing: Drawing::new(
            0,
            DrawingKind::TrendLine,
            0,
            vec![DrawingPoint {
                logical: 0.5,
                price: 10.0,
            }],
        ),
        preview: Some(DrawingPoint {
            logical: 1.0,
            price: 11.0,
        }),
        pane_constraint: None,
    });
    chart.drawing_controller.brush = Some(BrushCapture {
        pane_index: 0,
        points: vec![
            DrawingPoint {
                logical: 0.25,
                price: 10.0,
            },
            DrawingPoint {
                logical: 1.25,
                price: 11.0,
            },
        ],
        last_px: (f64::NAN, f64::NAN),
        options: Drawing::new(
            0,
            DrawingKind::Brush,
            0,
            vec![
                DrawingPoint {
                    logical: 0.75,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 1.75,
                    price: 11.0,
                },
            ],
        ),
    });

    assert!(chart.update_series_bar(0, 15.0, [10.5; 4]));

    assert_eq!(chart.drawing(id).unwrap().points[0].logical, 1.0);
    assert_eq!(
        chart.drawing_drag.as_ref().unwrap().start_points[1].logical,
        2.5
    );
    assert_eq!(
        chart
            .drawing_controller
            .pending
            .as_ref()
            .unwrap()
            .drawing
            .points[0]
            .logical,
        1.0
    );
    assert_eq!(
        chart
            .drawing_controller
            .pending
            .as_ref()
            .unwrap()
            .preview
            .unwrap()
            .logical,
        2.0
    );
    let capture = chart.drawing_controller.brush.as_ref().unwrap();
    assert_eq!(capture.points[0].logical, 0.5);
    assert_eq!(capture.points[1].logical, 2.25);
    assert_eq!(capture.options.points[0].logical, 1.5);
    assert!(capture.last_px.0.is_finite() && capture.last_px.1.is_finite());
    assert!(chart
        .drawing_drag
        .as_ref()
        .unwrap()
        .start_px
        .iter()
        .all(|(x, y)| x.is_finite() && y.is_finite()));
}

#[test]
fn active_drag_rebases_at_the_latest_pointer_without_a_followup_jump() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = [10.0, 20.0, 30.0];
    let values = [10.0, 11.0, 12.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![
                DrawingPoint {
                    logical: 0.5,
                    price: 10.25,
                },
                DrawingPoint {
                    logical: 1.5,
                    price: 11.25,
                },
            ],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));
    let start = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!(chart.drawing_drag_start_at(start.0, start.1));
    let pointer = (start.0 + 40.0, start.1 + 20.0);
    chart.drawing_drag_to(pointer.0, pointer.1, DrawingModifiers::default());

    assert!(chart.update_series_bar(0, 15.0, [100.0; 4]));
    chart.build_frame();
    let rebased = chart.drawing(id).unwrap().points.clone();
    chart.drawing_drag_to(pointer.0, pointer.1, DrawingModifiers::default());
    let unchanged = &chart.drawing(id).unwrap().points;
    assert_eq!(unchanged.len(), rebased.len());
    for (actual, expected) in unchanged.iter().zip(&rebased) {
        assert!((actual.logical - expected.logical).abs() < 1e-12);
        assert!((actual.price - expected.price).abs() < 1e-12);
    }

    chart.drawing_drag_cancel();
    assert_eq!(
        chart.drawing(id).unwrap().points,
        [
            DrawingPoint {
                logical: 1.0,
                price: 10.25,
            },
            DrawingPoint {
                logical: 2.5,
                price: 11.25,
            },
        ]
    );
}

#[test]
fn drawing_drag_is_one_history_entry_and_new_mutation_invalidates_redo() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let before = chart.drawing(id).unwrap().points.clone();
    let x = (x_at(&chart, before[0].logical) + x_at(&chart, before[1].logical)) / 2.0;
    let y = (y_at(&chart, before[0].price) + y_at(&chart, before[1].price)) / 2.0;
    assert!(chart.drawing_drag_start_at(x, y));
    for step in 1..=50 {
        chart.drawing_drag_to(
            x + f64::from(step),
            y + f64::from(step) / 2.0,
            DrawingModifiers::default(),
        );
    }
    chart.drawing_drag_end();
    let after = chart.drawing(id).unwrap().points.clone();
    assert_ne!(after, before);

    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
    // A second undo removes the creation, proving the 50 move samples coalesced into one entry.
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);

    assert!(chart.drawing_apply_options(id, r##"{"color":"#00ff00"}"##));
    assert!(!chart.can_redo_drawing());
    assert!(!chart.redo_drawing());
}

#[test]
fn cancelled_drag_rolls_back_and_keyboard_nudge_uses_history() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let before = chart.drawing(id).unwrap().points.clone();
    let x = (x_at(&chart, before[0].logical) + x_at(&chart, before[1].logical)) / 2.0;
    let y = (y_at(&chart, before[0].price) + y_at(&chart, before[1].price)) / 2.0;

    assert!(chart.drawing_drag_start_at(x, y));
    chart.drawing_drag_to(x + 40.0, y + 20.0, DrawingModifiers::default());
    assert_ne!(chart.drawing(id).unwrap().points, before);
    chart.drawing_drag_cancel();
    assert_eq!(chart.drawing(id).unwrap().points, before);

    chart.set_selected_drawing(Some(id));
    assert!(chart.nudge_selected_drawing(1.0, 0.0, None));
    assert_ne!(chart.drawing(id).unwrap().points, before);
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points, before);
}

#[test]
fn touch_profile_expands_anchor_hits_without_changing_precision_hits() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    chart.set_selected_drawing(Some(id));
    let point = chart.drawing(id).unwrap().points[0];
    let x = x_at(&chart, point.logical) + 15.0;
    let y = y_at(&chart, point.price) + 15.0;
    assert!(chart
        .hit_test_drawing_with_profile(x, y, HitProfile::PRECISION)
        .is_none());
    let touch = chart
        .hit_test_drawing_with_profile(x, y, HitProfile::TOUCH)
        .expect("44px touch anchor target");
    assert_eq!(touch.id, id);
    assert_eq!(touch.part, DrawingDragPart::Anchor(0));
}

#[test]
fn drawing_history_is_bounded_to_one_hundred_operations() {
    let mut chart = settled_chart();
    for offset in 0..101 {
        chart
            .add_drawing(
                DrawingKind::HorizontalLine,
                0,
                vec![DrawingPoint {
                    logical: f64::from(offset),
                    price: 10.0,
                }],
                None,
            )
            .unwrap();
    }
    let mut undone = 0;
    while chart.undo_drawing() {
        undone += 1;
    }
    assert_eq!(undone, 100);
    assert_eq!(chart.drawings().len(), 1);
}

#[test]
fn delete_undo_restores_prior_drawing_order() {
    let mut chart = settled_chart();
    let ids = (0..3)
        .map(|offset| {
            chart
                .add_drawing(
                    DrawingKind::HorizontalLine,
                    0,
                    vec![DrawingPoint {
                        logical: f64::from(offset),
                        price: 10.0 + f64::from(offset),
                    }],
                    None,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(chart.remove_drawing(ids[1]));
    assert!(chart.undo_drawing());
    assert_eq!(
        chart
            .drawings()
            .iter()
            .map(|drawing| drawing.id)
            .collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn add_drawing_validates_inputs() {
    let mut chart = settled_chart();
    // Wrong anchor counts are rejected.
    assert!(chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![DrawingPoint {
                logical: 1.0,
                price: 11.0
            }],
            None,
        )
        .is_none());
    assert!(chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![
                DrawingPoint {
                    logical: 1.0,
                    price: 11.0
                },
                DrawingPoint {
                    logical: 2.0,
                    price: 12.0
                },
            ],
            None,
        )
        .is_none());
    // A stale pane index is rejected.
    assert!(chart
        .add_drawing(
            DrawingKind::Text,
            7,
            vec![DrawingPoint {
                logical: 1.0,
                price: 11.0
            }],
            None,
        )
        .is_none());
    // Non-finite anchors are rejected.
    assert!(chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: f64::NAN,
                price: 11.0
            }],
            None,
        )
        .is_none());
    assert!(chart
        .add_drawing(
            DrawingKind::Path,
            0,
            vec![
                DrawingPoint {
                    logical: 1.0,
                    price: 11.0,
                };
                MAX_DRAWING_POINTS + 1
            ],
            None,
        )
        .is_none());
    assert_eq!(chart.drawings().len(), 0);
}

#[test]
fn options_patch_merges_and_serializes() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    assert!(chart.drawing_apply_options(
        id,
        r#"{"color":"rgb(200, 50, 100)","width":4,"style":"dashed","text":"hi",
            "text_h_align":"left","text_v_align":"top","text_bold":true,"text_size":18}"#,
    ));
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(id).unwrap()).unwrap();
    assert_eq!(options["color"], "rgb(200, 50, 100)");
    assert_eq!(options["width"], 4.0);
    assert_eq!(options["style"], "dashed");
    assert_eq!(options["text"], "hi");
    assert_eq!(options["text_h_align"], "left");
    assert_eq!(options["text_v_align"], "top");
    assert_eq!(options["text_bold"], true);
    assert_eq!(options["text_size"], 18.0);
    // Malformed JSON and unknown ids are host no-ops.
    assert!(!chart.drawing_apply_options(id, "{nope"));
    assert!(!chart.drawing_apply_options(999, "{}"));
    // Absent keys keep their values (reference merge semantics).
    assert!(chart.drawing_apply_options(id, r#"{"text":"bye"}"#));
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(id).unwrap()).unwrap();
    assert_eq!(options["text"], "bye");
    assert_eq!(options["width"], 4.0);
}

#[test]
fn b2_typed_schema_state_visibility_and_locked_contract() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let schema: serde_json::Value =
        serde_json::from_str(&chart.drawing_property_schema_json(id).unwrap()).unwrap();
    assert_eq!(schema["revision"], crate::DRAWING_CONTRACT_REVISION);
    assert!(schema["properties"]
        .as_array()
        .unwrap()
        .iter()
        .any(|property| property["name"] == "locked"));
    let kind_options: serde_json::Value =
        serde_json::from_str(&chart.drawing_kind_options_json(id).unwrap()).unwrap();
    assert_eq!(kind_options["kind"], "generic");
    assert!(chart.drawing_apply_options(
        id,
        r#"{"name":"risk","group_id":"orders","locked":true,"visible":false,"stroke_end":"arrow"}"#,
    ));
    assert!(!chart.drawing_is_visible(id));
    assert!(chart.set_drawing_visibility(id, true));
    assert!(chart.set_drawing_locked(id, true));
    let (x, y) = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!(!chart.drawing_drag_start_at(x, y));
    assert_eq!(chart.drawing(id).unwrap().name, "risk");
    assert_eq!(
        chart.drawing(id).unwrap().group_id.as_deref(),
        Some("orders")
    );
    let before = chart.drawing_options_json(id).unwrap();
    assert!(!chart.drawing_apply_options(
        id,
        &format!(
            r#"{{"name":"{}"}}"#,
            "x".repeat(crate::MAX_DRAWING_NAME_BYTES + 1)
        ),
    ));
    assert_eq!(chart.drawing_options_json(id).unwrap(), before);
}

#[test]
fn b2_clipboard_clone_and_z_order_are_bounded_and_undoable() {
    let mut chart = settled_chart();
    let first = add_trend(&mut chart);
    let second = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![DrawingPoint {
                logical: 4.0,
                price: 11.0,
            }],
            None,
        )
        .unwrap();
    let payload = chart.copy_drawings_json(&[first]).unwrap();
    let pasted = chart.paste_drawings_json(&payload, 0, 1.0, 0.5).unwrap();
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], first);
    assert!(chart.move_drawing_z_order(second, -1));
    assert!(chart.undo_drawing());
    assert!(chart.drawing(second).is_some());
}

#[test]
fn set_points_validates_count_and_finiteness() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    assert!(chart.drawing_set_points(
        id,
        r#"[{"logical":1.0,"price":10.0},{"logical":8.0,"price":13.0}]"#,
    ));
    let points: serde_json::Value =
        serde_json::from_str(&chart.drawing_points_json(id).unwrap()).unwrap();
    assert_eq!(points[1]["logical"], 8.0);
    assert!(!chart.drawing_set_points(id, r#"[{"logical":1.0,"price":10.0}]"#));
    assert!(!chart.drawing_set_points(id, "[1,2]"));
    assert!(!chart.drawing_set_points(999, "[]"));
}

/// A frame polyline's points, style, and line type.
type FramePolyline = (Vec<(f64, f64)>, LineStyle, LineType);

/// Every polyline of `color` in the first pane's frame.
fn frame_polylines(chart: &mut ChartEngine, color: Color) -> Vec<FramePolyline> {
    let frame = chart.build_frame();
    let pane = &frame.panes[0];
    pane.main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                style,
                line_type,
                color: stroke,
                ..
            } if *stroke == color => Some((
                pane.points[*first_point as usize..(*first_point + *point_count) as usize]
                    .iter()
                    .map(|point| (f64::from(point[0]), f64::from(point[1])))
                    .collect(),
                *style,
                *line_type,
            )),
            _ => None,
        })
        .collect()
}

/// Core drawings' dashed and dotted strokes (trend line, path, the curved brush) reach every
/// executor as solid dash runs lowered in the frame, like family strokes and series lines.
/// Solid strokes keep their single polyline, and a dashed line reaching far past the pane is
/// clipped before it is split, so its frame work stays bounded.
#[test]
fn dashed_core_drawing_strokes_reach_executors_as_solid_dash_runs() {
    let mut chart = settled_chart();
    let point = |logical: f64, price: f64| DrawingPoint { logical, price };
    let styled = |chart: &mut ChartEngine, kind, points, options: &str| {
        chart.add_drawing(kind, 0, points, Some(options)).unwrap()
    };
    styled(
        &mut chart,
        DrawingKind::TrendLine,
        vec![point(1.0, 10.2), point(8.0, 12.8)],
        r##"{"color":"#102030","style":"dashed","width":2}"##,
    );
    styled(
        &mut chart,
        DrawingKind::Path,
        vec![point(1.0, 12.0), point(4.0, 10.5), point(8.0, 12.5)],
        r##"{"color":"#203040","style":"dotted","width":2}"##,
    );
    styled(
        &mut chart,
        DrawingKind::Brush,
        vec![
            point(1.0, 11.0),
            point(3.0, 12.5),
            point(6.0, 10.2),
            point(8.5, 11.8),
        ],
        r##"{"color":"#304050","style":"dashed","width":3}"##,
    );
    styled(
        &mut chart,
        DrawingKind::TrendLine,
        vec![point(2.0, 10.0), point(7.0, 13.0)],
        r##"{"color":"#405060","width":2}"##,
    );
    let length = |points: &[(f64, f64)]| {
        points
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
            .sum::<f64>()
    };
    for (css, straight) in [("#102030", true), ("#203040", true), ("#304050", false)] {
        let color = Color::parse_css(css).unwrap();
        let runs = frame_polylines(&mut chart, color)
            .into_iter()
            // The path's open terminal chevron is its own solid three-point stroke.
            .filter(|(points, ..)| points.len() != 3 || !straight)
            .collect::<Vec<_>>();
        assert!(runs.len() > 5, "{css}: {} dash runs", runs.len());
        assert!(
            runs.iter()
                .all(|(_, style, line_type)| *style == LineStyle::Solid
                    && *line_type == LineType::Simple),
            "{css}: executors receive only solid straight runs"
        );
        // The gaps between dashes stay unpainted.
        let inked = runs.iter().map(|(points, ..)| length(points)).sum::<f64>();
        let first = runs.first().unwrap().0[0];
        let last = *runs.last().unwrap().0.last().unwrap();
        assert!(
            inked < (last.0 - first.0).hypot(last.1 - first.1).max(1.0) * 1.5,
            "{css}"
        );
    }
    let solid = frame_polylines(&mut chart, Color::parse_css("#405060").unwrap());
    assert_eq!(solid.len(), 1, "a solid trend line stays one polyline");
    assert_eq!((solid[0].0.len(), solid[0].1), (2, LineStyle::Solid));

    // A dashed line whose anchors sit a million bars off both pane edges splits only its
    // visible reach into dash runs.
    let far = styled(
        &mut chart,
        DrawingKind::TrendLine,
        vec![point(-1.0e6, 10.0), point(1.0e6, 13.0)],
        r##"{"color":"#506070","style":"dotted","width":1}"##,
    );
    let runs = frame_polylines(&mut chart, Color::parse_css("#506070").unwrap());
    assert!(
        !runs.is_empty() && runs.len() < 2_000,
        "{} runs for the far line",
        runs.len()
    );
    assert!(runs
        .iter()
        .flat_map(|(points, ..)| points)
        .all(|&(x, y)| (-20.0..=820.0).contains(&x) && (-20.0..=520.0).contains(&y)));
    assert!(chart.remove_drawing(far));
}

#[test]
fn trend_line_body_hit_and_miss() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    let hit = chart.hit_test_drawing(mx, my).unwrap();
    assert_eq!(hit.id, id);
    assert_eq!(hit.part, DrawingDragPart::Body);
    assert_eq!(hit.cursor, "move");
    // Within tolerance of the stroke (width 2 -> 1 + 3 px).
    assert!(chart.hit_test_drawing(mx, my + 3.5).is_some());
    // A clear miss far off the segment.
    assert!(chart.hit_test_drawing(mx, my + 12.0).is_none());
    // Past the segment's end (no extension beyond the anchors).
    assert!(chart.hit_test_drawing(x2 + 40.0, y2 - 20.0).is_none());
}

#[test]
fn horizontal_line_and_ray_hits() {
    let mut chart = settled_chart();
    let line = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![DrawingPoint {
                logical: 3.0,
                price: 11.0,
            }],
            None,
        )
        .unwrap();
    let ray = chart
        .add_drawing(
            DrawingKind::HorizontalRay,
            0,
            vec![DrawingPoint {
                logical: 6.0,
                price: 12.0,
            }],
            None,
        )
        .unwrap();
    let (line_y, ray_y) = (y_at(&chart, 11.0), y_at(&chart, 12.0));
    // The full-width line hits at any x.
    assert_eq!(chart.hit_test_drawing(10.0, line_y).unwrap().id, line);
    assert_eq!(chart.hit_test_drawing(790.0, line_y).unwrap().id, line);
    // The ray hits only right of its anchor (it is topmost, so it wins near its level).
    let anchor_x = x_at(&chart, 6.0);
    assert_eq!(
        chart.hit_test_drawing(anchor_x + 30.0, ray_y).unwrap().id,
        ray
    );
    assert!(chart.hit_test_drawing(anchor_x - 30.0, ray_y).is_none());

    // A right ray whose origin is already beyond the right pane edge has no visible body. The
    // resolved geometry must retain that direction instead of normalizing it into a backwards
    // finite segment at the pane edge.
    let offscreen_ray = chart
        .add_drawing(
            DrawingKind::HorizontalRay,
            0,
            vec![DrawingPoint {
                logical: 10_000.0,
                price: 13.0,
            }],
            None,
        )
        .unwrap();
    let offscreen_px = chart
        .drawing_px(chart.drawing(offscreen_ray).unwrap())
        .unwrap()[0]
        .0;
    assert!(offscreen_px > chart.pane_w);
    assert!(chart
        .hit_test_drawing(chart.pane_w - 1.0, y_at(&chart, 13.0))
        .is_none());
}

#[test]
fn vertical_line_hit() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::VerticalLine,
            0,
            vec![DrawingPoint {
                logical: 4.0,
                price: 0.0,
            }],
            None,
        )
        .unwrap();
    let x = x_at(&chart, 4.0);
    assert_eq!(chart.hit_test_drawing(x, 60.0).unwrap().id, id);
    assert_eq!(chart.hit_test_drawing(x, 440.0).unwrap().id, id);
    assert!(chart.hit_test_drawing(x + 8.0, 60.0).is_none());
}

#[test]
fn body_drag_with_ctrl_magnets_single_anchor_lines() {
    let mut chart = settled_chart();
    let magnet = DrawingModifiers {
        magnet: true,
        straighten: false,
    };
    // A vertical line dragged by its body with Ctrl snaps its x to the nearest bar center.
    let vline = chart
        .add_drawing(
            DrawingKind::VerticalLine,
            0,
            vec![DrawingPoint {
                logical: 2.3,
                price: 0.0,
            }],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(vline));
    let x = chart.time_scale.logical_to_coordinate(2.3);
    assert!(chart.drawing_drag_start_at(x, 250.0));
    chart.drawing_drag_to(chart.time_scale.logical_to_coordinate(5.4), 250.0, magnet);
    chart.drawing_drag_end();
    let d = chart.drawings.iter().find(|d| d.id == vline).unwrap();
    assert_eq!(d.points[0].logical, 5.0, "snapped to bar 5's center");

    // A horizontal line's body drag with Ctrl snaps its price to the nearest OHLC of the bar
    // under the cursor.
    let hline = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![DrawingPoint {
                logical: 2.0,
                price: 10.9,
            }],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(hline));
    let y = y_at(&chart, 10.9);
    assert!(chart.drawing_drag_start_at(x_at(&chart, 4.0), y));
    chart.drawing_drag_to(x_at(&chart, 4.0), y_at(&chart, 11.9), magnet);
    chart.drawing_drag_end();
    let d = chart.drawings.iter().find(|d| d.id == hline).unwrap();
    // Bar 4's OHLC all read 11.0 in the settled fixture — the only snap target.
    assert_eq!(d.points[0].price, 11.0, "snapped to the bar's value");
}

#[test]
fn the_creation_preview_shows_all_eight_rectangle_anchors() {
    let mut chart = settled_chart();
    assert!(chart.drawing_create_begin(DrawingKind::Rectangle, None));
    // First corner committed; the preview follows the cursor — all eight handles paint
    // immediately (the public reference), not only after the second click commits.
    // First corner committed (the -1 "awaiting more anchors" result), the preview follows the
    // cursor — all eight handles paint immediately (the public reference), not only after the commit.
    chart.drawing_create_click(
        x_at(&chart, 2.0),
        y_at(&chart, 10.0),
        DrawingModifiers::default(),
    );
    chart.drawing_create_move(
        x_at(&chart, 6.0),
        y_at(&chart, 12.0),
        DrawingModifiers::default(),
    );
    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    let circles = main
        .iter()
        .filter(|p| matches!(p, Prim::Circle { .. }))
        .count();
    let rounds = main
        .iter()
        .filter(|p| matches!(p, Prim::RoundRect { .. }))
        .count();
    assert_eq!(circles, 8, "four corner discs during the preview");
    assert_eq!(rounds, 8, "four midpoint squares during the preview");
    chart.drawing_create_cancel();
}

#[test]
fn a_selected_rectangle_paints_eight_handles_and_a_styleable_border() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.0,
                },
            ],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));

    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    let drawing_color = Color::parse_css(DRAWING_DEFAULT_COLOR).unwrap();
    let circles = main
        .iter()
        .filter(|p| matches!(p, Prim::Circle { .. }))
        .count();
    let rounds = main
        .iter()
        .filter(|p| matches!(p, Prim::RoundRect { .. }))
        .count();
    assert_eq!(
        circles, 8,
        "four corner discs (border + fill each), got {circles}"
    );
    assert_eq!(
        rounds, 8,
        "four midpoint squares (border + fill each), got {rounds}"
    );
    assert!(!chart.drawing(id).unwrap().border_visible);
    assert!(
        !main
            .iter()
            .any(|p| matches!(p, Prim::RectFrame { color, .. } if *color == drawing_color)),
        "the default rectangle has no border"
    );

    // Dotted/dashed borders: the frame becomes four crisp line prims in the drawing color.
    assert!(chart.drawing_apply_options(id, r#"{"style":"dotted","border_visible":true}"#));
    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    assert!(
        !main
            .iter()
            .any(|p| matches!(p, Prim::RectFrame { color, .. } if *color == drawing_color)),
        "no solid frame under a dotted border"
    );
    let dotted_lines = main
        .iter()
        .filter(|p| {
            matches!(
                p,
                Prim::HLine { style: LineStyle::Dotted, color, .. }
                | Prim::VLine { style: LineStyle::Dotted, color, .. }
                if *color == drawing_color
            )
        })
        .count();
    assert_eq!(dotted_lines, 4, "four dotted border segments");
}

#[test]
fn rectangle_border_default_and_explicit_override_survive_persistence() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.0,
                },
            ],
            None,
        )
        .unwrap();
    let saved = chart.export_state_json().unwrap();
    assert!(saved.contains("\"border_visible\":false"));
    let mut restored = settled_chart();
    restored.import_state_json(&saved).unwrap();
    assert!(!restored.drawing(id).unwrap().border_visible);

    assert!(chart.drawing_apply_options(id, r#"{"border_visible":true}"#));
    let saved = chart.export_state_json().unwrap();
    assert!(saved.contains("\"border_visible\":true"));
    let mut restored = settled_chart();
    restored.import_state_json(&saved).unwrap();
    assert!(restored.drawing(id).unwrap().border_visible);

    let mut legacy: serde_json::Value = serde_json::from_str(&saved).unwrap();
    legacy["drawings"][0]["style"]
        .as_object_mut()
        .unwrap()
        .remove("border_visible");
    let mut restored = settled_chart();
    restored.import_state_json(&legacy.to_string()).unwrap();
    assert!(restored.drawing(id).unwrap().border_visible);
}

#[test]
fn rectangle_body_hits_the_frame_band_not_the_middle() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.0,
                },
            ],
            None,
        )
        .unwrap();
    let (left, right) = (x_at(&chart, 2.0), x_at(&chart, 6.0));
    let (bottom, top) = (y_at(&chart, 10.0), y_at(&chart, 12.0));
    // the public reference: the fill is not a drag surface while UNSELECTED — the middle misses (the
    // chart pans there); once selected (a border click), the middle moves the drawing.
    assert!(chart
        .hit_test_drawing((left + right) / 2.0, (top + bottom) / 2.0)
        .is_none());
    chart.set_selected_drawing(Some(id));
    let mid = chart.hit_test_drawing((left + right) / 2.0, (top + bottom) / 2.0);
    assert_eq!(
        mid,
        Some(DrawingHit {
            id,
            part: DrawingDragPart::Body,
            cursor: "move"
        })
    );
    chart.set_selected_drawing(None);
    // The border frame and just outside it (within tolerance) grab the body.
    assert_eq!(chart.hit_test_drawing(left, top).unwrap().id, id);
    assert_eq!(chart.hit_test_drawing(right, bottom).unwrap().id, id);
    assert_eq!(
        chart
            .hit_test_drawing(left, (top + bottom) / 2.0)
            .unwrap()
            .id,
        id
    );
    assert_eq!(
        chart
            .hit_test_drawing((left + right) / 2.0, bottom)
            .unwrap()
            .id,
        id
    );
    assert!(chart.hit_test_drawing(left - 2.0, top).is_some());
    // Clear misses outside the box.
    assert!(chart.hit_test_drawing(left - 12.0, top).is_none());
    assert!(chart.hit_test_drawing(right + 12.0, bottom).is_none());
}

#[test]
fn rectangle_shows_eight_anchors_with_directional_cursors() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.0,
                },
            ],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));
    let (left, right) = (x_at(&chart, 2.0), x_at(&chart, 6.0));
    let (bottom, top) = (y_at(&chart, 10.0), y_at(&chart, 12.0));
    let (mx, my) = ((left + right) / 2.0, (top + bottom) / 2.0);
    // Clock order from the top-left: corners get diagonal cursors, midpoints straight ones.
    let cases: [((f64, f64), usize, &str); 8] = [
        ((left, top), 0, "nwse-resize"),
        ((mx, top), 1, "ns-resize"),
        ((right, top), 2, "nesw-resize"),
        ((right, my), 3, "ew-resize"),
        ((right, bottom), 4, "nwse-resize"),
        ((mx, bottom), 5, "ns-resize"),
        ((left, bottom), 6, "nesw-resize"),
        ((left, my), 7, "ew-resize"),
    ];
    for ((x, y), index, cursor) in cases {
        let hit = chart.hit_test_drawing(x, y).unwrap();
        assert_eq!(hit.part, DrawingDragPart::Anchor(index), "anchor {index}");
        assert_eq!(hit.cursor, cursor, "anchor {index} cursor");
    }
    // Unselected, the same anchor point hits nothing (the middle is not a body surface).
    chart.set_selected_drawing(None);
    assert!(chart.hit_test_drawing(mx, top).is_some()); // the frame band still bodies
}

#[test]
fn rectangle_anchor_drags_resize_independently_and_flip_across_the_opposite_side() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.0,
                },
            ],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));
    let (left, right) = (x_at(&chart, 2.0), x_at(&chart, 6.0));
    let (bottom, top) = (y_at(&chart, 10.0), y_at(&chart, 12.0));
    let mx = (left + right) / 2.0;
    let box_of = |chart: &ChartEngine| {
        let d = chart.drawings.iter().find(|d| d.id == id).unwrap();
        let px = chart.drawing_px(d).unwrap();
        (
            px[0].0.min(px[1].0),
            px[0].1.min(px[1].1),
            px[0].0.max(px[1].0),
            px[0].1.max(px[1].1),
        )
    };

    // Top-mid anchor (1) drag: only the top edge moves; x sides stay.
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6; // px→logical→px round-trip fuzz
    assert!(chart.drawing_drag_start_at(mx, top));
    chart.drawing_drag_to(mx + 30.0, top - 40.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let (l, t, r, b) = box_of(&chart);
    assert!(close(l, left) && close(r, right));
    assert!(close(t, top - 40.0));
    assert!(close(b, bottom));

    // Top-mid dragged BELOW the bottom edge: the box flips — the bottom edge holds, the
    // dragged edge becomes the new bottom.
    assert!(chart.drawing_drag_start_at(mx, t));
    chart.drawing_drag_to(mx, bottom + 50.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let (l, t2, r, b2) = box_of(&chart);
    assert!(close(l, left) && close(r, right));
    assert!(
        close(t2, bottom),
        "the opposite edge stays put through the flip"
    );
    assert!(close(b2, bottom + 50.0));

    // Corner (4 = bottom-right) drag past the top-left: both axes flip independently.
    assert!(chart.drawing_drag_start_at(r, b2));
    chart.drawing_drag_to(left - 60.0, t2 - 60.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let (l3, t3, r3, b3) = box_of(&chart);
    assert!(
        close(r3, left),
        "the fixed corner side holds through the flip"
    );
    assert!(close(b3, t2));
    assert!(close(l3, left - 60.0));
    assert!(close(t3, t2 - 60.0));
}

#[test]
fn text_hit_uses_measured_box() {
    let mut chart = settled_chart();
    // Host-style measure hook: 0.5 em per glyph (replaces the 0.6 estimate).
    chart.set_text_measure(Some(Box::new(|text, size, _family, _weight, _italic| {
        text.chars().count() as f64 * size * 0.5
    })));
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r#"{"text":"abcd","text_size":20}"#),
        )
        .unwrap();
    let (ax, ay) = (x_at(&chart, 5.0), y_at(&chart, 11.0));
    // Center/middle placement: the 40×24 box centers on the anchor, plus the container's
    // 4 px padding and 2 px editing border on every side.
    assert_eq!(chart.hit_test_drawing(ax, ay).unwrap().id, id);
    assert_eq!(chart.hit_test_drawing(ax + 19.0, ay).unwrap().id, id);
    assert_eq!(chart.hit_test_drawing(ax + 23.0, ay).unwrap().id, id);
    assert_eq!(chart.hit_test_drawing(ax + 25.0, ay).unwrap().id, id);
    assert!(chart.hit_test_drawing(ax + 27.0, ay).is_none());
    assert_eq!(chart.hit_test_drawing(ax, ay + 17.0).unwrap().id, id);
    assert!(chart.hit_test_drawing(ax, ay + 19.0).is_none());
}

#[test]
fn anchors_hit_only_when_selected() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    // Unselected: the anchor point hits as Body (it lies on the stroke).
    let hit = chart.hit_test_drawing(x1, y1).unwrap();
    assert_eq!(hit.part, DrawingDragPart::Body);
    // Selected: the same point hits as Anchor(0) with the pointer cursor.
    chart.set_selected_drawing(Some(id));
    let hit = chart.hit_test_drawing(x1, y1).unwrap();
    assert_eq!(hit.part, DrawingDragPart::Anchor(0));
    assert_eq!(hit.cursor, "pointer");
    // A point on the stroke outside the handle radius hits the body, not the anchor.
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    let on_stroke = (x1 + (x2 - x1) * 0.05, y1 + (y2 - y1) * 0.05);
    let hit = chart.hit_test_drawing(on_stroke.0, on_stroke.1).unwrap();
    assert_eq!(hit.part, DrawingDragPart::Body);
}

#[test]
fn topmost_drawing_wins() {
    let mut chart = settled_chart();
    let first = add_trend(&mut chart);
    // A second trend line directly over the first (later = on top).
    let second = add_trend(&mut chart);
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    let hit = chart
        .hit_test_drawing((x1 + x2) / 2.0, (y1 + y2) / 2.0)
        .unwrap();
    assert_eq!(hit.id, second);
    chart.remove_drawing(second);
    let hit = chart
        .hit_test_drawing((x1 + x2) / 2.0, (y1 + y2) / 2.0)
        .unwrap();
    assert_eq!(hit.id, first);
}

#[test]
fn anchor_drag_reanchors_one_point() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    chart.set_selected_drawing(Some(id));
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    assert!(chart.drawing_drag_start_at(x1, y1));
    // Drag anchor 0 to logical 4 / price 12.
    chart.drawing_drag_to(
        x_at(&chart, 4.0),
        y_at(&chart, 12.0),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!((points[0].logical - 4.0).abs() < 1e-6);
    assert!((points[0].price - 12.0).abs() < 1e-9);
    // Anchor 1 untouched.
    assert_eq!(points[1].logical, 7.0);
    assert_eq!(points[1].price, 12.5);
    assert!(!chart.drawing_drag_active());
}

#[test]
fn position_controls_have_dedicated_target_entry_extent_and_stop_drag_semantics() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 11.5,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 12.5,
                },
                DrawingPoint {
                    // Stop x is deliberately different: position geometry must not turn it into
                    // a third horizontal corner.
                    logical: 5.0,
                    price: 10.5,
                },
            ],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(id));

    let handles = [
        (x_at(&chart, 2.0), y_at(&chart, 12.5), 0, "ns-resize"),
        (x_at(&chart, 2.0), y_at(&chart, 11.5), 1, "move"),
        (x_at(&chart, 7.0), y_at(&chart, 11.5), 2, "ew-resize"),
        (x_at(&chart, 2.0), y_at(&chart, 10.5), 3, "ns-resize"),
    ];
    for (x, y, index, cursor) in handles {
        let hit = chart.hit_test_drawing(x, y).expect("position control hit");
        assert_eq!(hit.part, DrawingDragPart::Anchor(index));
        assert_eq!(hit.cursor, cursor);
    }

    // Target moves vertically only.
    assert!(chart.drawing_drag_start_at(x_at(&chart, 2.0), y_at(&chart, 12.5)));
    chart.drawing_drag_to(
        x_at(&chart, 4.0),
        y_at(&chart, 12.75),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(points[1].logical, 7.0);
    assert!((points[1].price - 12.75).abs() < 1e-9);

    // Horizontal extent moves horizontally only.
    assert!(chart.drawing_drag_start_at(x_at(&chart, 7.0), y_at(&chart, 11.5)));
    chart.drawing_drag_to(
        x_at(&chart, 8.0),
        y_at(&chart, 12.0),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!((points[1].logical - 8.0).abs() < 1e-6);
    assert!((points[1].price - 12.75).abs() < 1e-9);

    // Entry/origin moves the origin edge and entry level without moving target/stop prices.
    assert!(chart.drawing_drag_start_at(x_at(&chart, 2.0), y_at(&chart, 11.5)));
    chart.drawing_drag_to(
        x_at(&chart, 3.0),
        y_at(&chart, 11.25),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!((points[0].logical - 3.0).abs() < 1e-6);
    assert!((points[0].price - 11.25).abs() < 1e-9);
    assert!((points[2].logical - 3.0).abs() < 1e-6);
    assert!((points[1].price - 12.75).abs() < 1e-9);
    assert!((points[2].price - 10.5).abs() < 1e-9);

    // Stop moves vertically only.
    assert!(chart.drawing_drag_start_at(x_at(&chart, 3.0), y_at(&chart, 10.5)));
    chart.drawing_drag_to(
        x_at(&chart, 6.0),
        y_at(&chart, 10.25),
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!((points[2].logical - 3.0).abs() < 1e-6);
    assert!((points[2].price - 10.25).abs() < 1e-9);
}

#[test]
fn position_drag_resolves_price_ticks_smaller_than_a_device_pixel() {
    use aeris_charts_core::model::price_range::PriceRange;

    for kind in [DrawingKind::LongPosition, DrawingKind::ShortPosition] {
        for dpr in [1.0, 1.5, 2.0] {
            for part in [
                DrawingDragPart::Body,
                DrawingDragPart::Anchor(0),
                DrawingDragPart::Anchor(1),
                DrawingDragPart::Anchor(3),
            ] {
                let mut chart = settled_chart();
                chart.dpr = dpr;
                chart.panes[0]
                    .price_scale
                    .set_price_range(Some(PriceRange::new(0.0, 30.0)));
                let direction = if kind == DrawingKind::LongPosition {
                    1.0
                } else {
                    -1.0
                };
                let initial = vec![
                    DrawingPoint {
                        logical: 2.0,
                        price: 11.5,
                    },
                    DrawingPoint {
                        logical: 7.0,
                        price: 11.5 + direction,
                    },
                    DrawingPoint {
                        logical: 2.0,
                        price: 11.5 - direction,
                    },
                ];
                let id = chart.add_drawing(kind, 0, initial.clone(), None).unwrap();
                chart.set_selected_drawing(Some(id));
                let px = chart.drawing_px(chart.drawing(id).unwrap()).unwrap();
                let (grab, changed) = match part {
                    DrawingDragPart::Body => ((px[0].0 + 30.0, px[0].1), None),
                    DrawingDragPart::Anchor(0) => ((px[0].0, px[1].1), Some(1)),
                    DrawingDragPart::Anchor(1) => (px[0], Some(0)),
                    DrawingDragPart::Anchor(3) => (px[2], Some(2)),
                    _ => unreachable!(),
                };
                let price = initial[changed.unwrap_or(0)].price;
                let tick_px = y_at(&chart, price + 0.01) - y_at(&chart, price);
                assert!(
                    tick_px.abs() * dpr < 1.0,
                    "fixture must exercise subpixel price ticks"
                );
                assert!(chart.drawing_drag_start_at(grab.0, grab.1));
                assert_eq!(chart.drawing_drag.as_ref().unwrap().part, part);
                chart.drawing_drag_to(grab.0, grab.1 + tick_px * 0.3, DrawingModifiers::default());
                for (before, after) in initial.iter().zip(&chart.drawing(id).unwrap().points) {
                    assert!(
                        (after.price - before.price).abs() < 1e-9,
                        "below half a tick: {kind:?}, {part:?}, DPR {dpr}"
                    );
                }
                chart.drawing_drag_to(grab.0, grab.1 + tick_px * 0.7, DrawingModifiers::default());
                for (index, (before, after)) in initial
                    .iter()
                    .zip(&chart.drawing(id).unwrap().points)
                    .enumerate()
                {
                    let expected = before.price
                        + if changed.is_none() || changed == Some(index) {
                            0.01
                        } else {
                            0.0
                        };
                    assert!(
                        (after.price - expected).abs() < 1e-9,
                        "one exact tick: {kind:?}, {part:?}, DPR {dpr}, {after:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn position_horizontal_drag_follows_crosshair_steps_in_both_directions() {
    for kind in [DrawingKind::LongPosition, DrawingKind::ShortPosition] {
        for spacing in [6.0, 14.25] {
            for part in [
                DrawingDragPart::Body,
                DrawingDragPart::Anchor(1),
                DrawingDragPart::Anchor(2),
            ] {
                let mut chart = settled_chart();
                chart.time_scale.set_bar_spacing(spacing);
                chart.time_scale.set_right_offset(5.0);
                let sign = if kind == DrawingKind::LongPosition {
                    1.0
                } else {
                    -1.0
                };
                let initial = vec![
                    DrawingPoint {
                        logical: 2.0,
                        price: 11.5,
                    },
                    DrawingPoint {
                        logical: 7.0,
                        price: 11.5 + sign,
                    },
                    DrawingPoint {
                        logical: 2.0,
                        price: 11.5 - sign,
                    },
                ];
                let id = chart.add_drawing(kind, 0, initial.clone(), None).unwrap();
                chart.set_selected_drawing(Some(id));
                let px = chart.drawing_px(chart.drawing(id).unwrap()).unwrap();
                let grab = match part {
                    DrawingDragPart::Body => (px[0].0 + (px[1].0 - px[0].0) * 0.37, px[0].1),
                    DrawingDragPart::Anchor(1) => px[0],
                    DrawingDragPart::Anchor(2) => (px[1].0, px[0].1),
                    _ => unreachable!(),
                };
                assert!(chart.drawing_drag_start_at(grab.0, grab.1));
                assert_eq!(chart.drawing_drag.as_ref().unwrap().part, part);
                let cursor_start = chart.snapped_crosshair_index(grab.0);
                for fraction in [0.2, 0.4, 0.7, 1.2, 1.7, -0.2, -0.4, -0.7, -1.2, -1.7, 8.7] {
                    let x = grab.0 + spacing * fraction;
                    let cursor = chart.snapped_crosshair_index(x);
                    chart.drawing_drag_to(x, grab.1, DrawingModifiers::default());
                    for (index, (before, after)) in initial
                        .iter()
                        .zip(&chart.drawing(id).unwrap().points)
                        .enumerate()
                    {
                        let expected = match part {
                            DrawingDragPart::Body => {
                                before.logical + (cursor - cursor_start) as f64
                            }
                            DrawingDragPart::Anchor(1) if index != 1 => cursor as f64,
                            DrawingDragPart::Anchor(2) if index == 1 => cursor as f64,
                            _ => before.logical,
                        };
                        assert!((after.logical - expected).abs() < 1e-9, "{kind:?} {part:?}, spacing {spacing}, movement {fraction}: {after:?}, cursor {cursor}");
                        assert!((after.price - before.price).abs() < 1e-9);
                    }
                }
                assert!(
                    chart
                        .drawing(id)
                        .unwrap()
                        .points
                        .iter()
                        .any(|point| point.logical > 9.0),
                    "time-slot snapping must allow future empty space"
                );
            }
        }
    }
}

#[test]
fn position_creation_and_drag_use_instrument_and_crosshair_ticks() {
    for kind in [DrawingKind::LongPosition, DrawingKind::ShortPosition] {
        let mut chart = settled_chart();
        chart
            .set_instrument_metadata(crate::InstrumentMetadata {
                tick_size: Some(0.25),
                ..Default::default()
            })
            .unwrap();
        assert!(chart.set_drawing_tool(Some(kind), None, None));
        let id = chart
            .drawing_tool_activate(
                x_at(&chart, 4.0) + (x_at(&chart, 5.0) - x_at(&chart, 4.0)) * 0.24,
                y_at(&chart, 11.4),
                DrawingModifiers::default(),
            )
            .created
            .unwrap();
        let initial = chart.drawing(id).unwrap().points.clone();
        assert_eq!(initial[0].logical, 4.0);
        assert!(initial.iter().all(|point| point.logical.fract() == 0.0));
        assert!(initial
            .iter()
            .all(|point| (point.price / 0.25 - (point.price / 0.25).round()).abs() < 1e-9));
        let risk = (initial[0].price - initial[2].price).abs();
        let reward = (initial[1].price - initial[0].price).abs();
        assert!(risk > 0.0 && (reward / risk - 2.0).abs() < 1e-9);
        let entry = chart.drawing_px(chart.drawing(id).unwrap()).unwrap()[0];
        let tick_px = y_at(&chart, initial[0].price + 0.25) - entry.1;
        let dx = (x_at(&chart, 5.0) - x_at(&chart, 4.0)) * 0.4;
        let cursor_start = chart.snapped_crosshair_index(entry.0 + 30.0);
        let cursor_end = chart.snapped_crosshair_index(entry.0 + 30.0 + dx);
        assert!(chart.drawing_drag_start_at(entry.0 + 30.0, entry.1));
        chart.drawing_drag_to(
            entry.0 + 30.0 + dx,
            entry.1 + tick_px * 0.7,
            DrawingModifiers::default(),
        );
        for (before, after) in initial.iter().zip(&chart.drawing(id).unwrap().points) {
            assert!((after.price - before.price - 0.25).abs() < 1e-9);
            assert!(
                (after.logical - before.logical - (cursor_end - cursor_start) as f64).abs() < 1e-6
            );
        }
    }
}

#[test]
fn position_levels_follow_the_price_band_ladder_of_their_scale() {
    let mut chart = settled_chart();
    // A ladder is the scale's single tick source: 0.01 below 10 and 0.02 from 10 up, whatever
    // scalar tick the instrument or the display format carries.
    chart
        .set_instrument_metadata(crate::InstrumentMetadata {
            tick_size: Some(0.25),
            ..Default::default()
        })
        .unwrap();
    assert!(chart.series_apply_price_format_json(
        0,
        r#"{"type":"price","tick_ladder":[{"from":0,"min_move":0.01},{"from":10,"min_move":0.02}]}"#
    ));
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 12.0,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 13.0,
                },
                DrawingPoint {
                    logical: 2.0,
                    price: 11.0,
                },
            ],
            None,
        )
        .unwrap();
    let scale = DrawingPriceScale::Right;
    // Prices snap to the band tick, not the instrument tick.
    assert!((chart.snap_position_price(0, scale, 11.013) - 11.02).abs() < 1e-9);
    assert!((chart.snap_position_price(0, scale, 9.987) - 9.99).abs() < 1e-9);
    // Ticks count the ladder's cumulative band ticks: 100 of 0.01 below 10, then 50 of 0.02.
    assert_eq!(
        chart.position_price_ticks_between(0, scale, 9.0, 11.0),
        Some(150.0)
    );
    assert_eq!(chart.position_price_tick_at(0, scale, 9.5), Some(0.01));
    assert_eq!(chart.position_price_tick_at(0, scale, 11.0), Some(0.02));
    // A keyboard nudge far below one tick still steps one band tick the key's way.
    chart.set_selected_drawing(Some(id));
    assert!(chart.nudge_selected_drawing(0.0, 1.0, Some(3)));
    let stop = chart.drawing(id).unwrap().points[2].price;
    assert!((stop - 10.98).abs() < 1e-9, "stop {stop}");
}

#[test]
fn body_drag_moves_all_points_by_the_same_delta() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    // Grab the segment's middle: a body drag.
    let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    assert!(chart.drawing_drag_start_at(mx, my));
    assert_eq!(chart.selected_drawing(), Some(id)); // grabbing selects
                                                    // Move two bars right and one price unit up (up = negative y).
    let y_unit = y_at(&chart, 11.0) - y_at(&chart, 12.0);
    let dx = x_at(&chart, 4.0) - x_at(&chart, 2.0);
    chart.drawing_drag_to(mx + dx, my - y_unit, DrawingModifiers::default());
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!((points[0].logical - 4.0).abs() < 1e-6);
    assert!((points[1].logical - 9.0).abs() < 1e-6);
    assert!((points[0].price - 11.5).abs() < 1e-9);
    assert!((points[1].price - 13.5).abs() < 1e-9);
}

#[test]
fn full_span_kinds_freeze_the_unused_axis_in_drags() {
    let mut chart = settled_chart();
    let hline = chart
        .add_drawing(
            DrawingKind::HorizontalLine,
            0,
            vec![DrawingPoint {
                logical: 3.0,
                price: 11.0,
            }],
            None,
        )
        .unwrap();
    let line_y = y_at(&chart, 11.0);
    // A diagonal body drag moves the line only vertically.
    assert!(chart.drawing_drag_start_at(200.0, line_y));
    chart.drawing_drag_to(500.0, y_at(&chart, 12.0), DrawingModifiers::default());
    chart.drawing_drag_end();
    let point = chart.drawing(hline).unwrap().points[0];
    assert_eq!(point.logical, 3.0);
    assert!((point.price - 12.0).abs() < 1e-9);

    let vline = chart
        .add_drawing(
            DrawingKind::VerticalLine,
            0,
            vec![DrawingPoint {
                logical: 4.0,
                price: 0.0,
            }],
            None,
        )
        .unwrap();
    let x = x_at(&chart, 4.0);
    println!("x={} hit={:?}", x, chart.hit_test_drawing(x, 250.0));
    assert!(chart.drawing_drag_start_at(x, 250.0));
    chart.drawing_drag_to(x_at(&chart, 6.0), 400.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    let point = chart.drawing(vline).unwrap().points[0];
    assert!((point.logical - 6.0).abs() < 1e-6);
    assert_eq!(point.price, 0.0);
}

#[test]
fn creation_flow_commits_after_the_kinds_anchor_count() {
    let mut chart = settled_chart();
    // Not armed: clicks and moves are no-ops.
    assert_eq!(
        chart.drawing_create_click(100.0, 100.0, DrawingModifiers::default()),
        0
    );
    assert!(chart.drawing_create_begin(DrawingKind::TrendLine, Some(r##"{"color":"#ff0000"}"##)));
    assert!(chart.drawing_create_active());
    // First click: pending (-1), and the move preview tracks the mouse.
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    assert_eq!(
        chart.drawing_create_click(x1, y1, DrawingModifiers::default()),
        -1
    );
    chart.drawing_create_move(
        x_at(&chart, 5.0),
        y_at(&chart, 11.0),
        DrawingModifiers::default(),
    );
    let pending = chart.pending_drawing().unwrap();
    assert_eq!(pending.drawing.points.len(), 1);
    let preview = pending.preview.unwrap();
    assert!((preview.logical - 5.0).abs() < 1e-6);
    assert!((preview.price - 11.0).abs() < 1e-6);
    // Second click commits; the new drawing is left selected.
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    let id = chart.drawing_create_click(x2, y2, DrawingModifiers::default());
    assert!(id > 0);
    let id = id as DrawingId;
    assert!(!chart.drawing_create_active());
    assert_eq!(chart.selected_drawing(), Some(id));
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.kind, DrawingKind::TrendLine);
    assert_eq!(drawing.color, "#ff0000");
    assert!((drawing.points[0].logical - 2.0).abs() < 1e-6);
    assert!((drawing.points[1].price - 12.5).abs() < 1e-9);
}

#[test]
fn position_tools_commit_complete_non_overlapping_geometry_on_one_click() {
    let mut chart = settled_chart();
    let click = (x_at(&chart, 4.0), y_at(&chart, 11.5));

    for kind in [DrawingKind::LongPosition, DrawingKind::ShortPosition] {
        assert!(chart.set_drawing_tool(Some(kind), None, None));
        let update = chart.drawing_tool_activate(click.0, click.1, DrawingModifiers::default());
        let id = update
            .created
            .expect("position commits on the first activation");
        assert_eq!(chart.active_drawing_tool(), None);
        assert!(!chart.drawing_create_active());

        let drawing = chart.drawing(id).unwrap();
        assert_eq!(drawing.kind, kind);
        assert_eq!(drawing.points.len(), 3);
        let entry = drawing.points[0];
        let target = drawing.points[1];
        let stop = drawing.points[2];
        assert_eq!(stop.logical, entry.logical, "stop stays on the origin edge");
        assert_ne!(
            target.logical, entry.logical,
            "position has an editable width"
        );
        match kind {
            DrawingKind::LongPosition => {
                assert!(target.price > entry.price);
                assert!(stop.price < entry.price);
            }
            DrawingKind::ShortPosition => {
                assert!(target.price < entry.price);
                assert!(stop.price > entry.price);
            }
            _ => unreachable!(),
        }
        let reward_distance = (target.price - entry.price).abs();
        let risk_distance = (entry.price - stop.price).abs();
        assert!(risk_distance > 0.0);
        assert!(
            ((reward_distance / risk_distance) - 2.0).abs() < 1e-9,
            "single-click position preset must open at an asymmetric 2:1 reward/risk ratio"
        );
    }
}

#[test]
fn position_input_normalization_prevents_same_side_or_nested_regions() {
    let mut chart = settled_chart();
    let long = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 11.0,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 9.0,
                },
                DrawingPoint {
                    logical: 5.0,
                    price: 13.0,
                },
            ],
            None,
        )
        .unwrap();
    let points = &chart.drawing(long).unwrap().points;
    assert!(points[1].price > points[0].price);
    assert!(points[2].price < points[0].price);
    assert_eq!(points[2].logical, points[0].logical);

    assert!(chart.drawing_set_points(
        long,
        r#"[{"logical":3,"price":11},{"logical":8,"price":10},{"logical":6,"price":12}]"#,
    ));
    let points = &chart.drawing(long).unwrap().points;
    assert!(points[1].price > points[0].price);
    assert!(points[2].price < points[0].price);
    assert_eq!(points[2].logical, points[0].logical);
}

#[test]
fn drawing_tool_controller_routes_placement_classes_without_host_kind_branches() {
    let mut chart = settled_chart();
    let first = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let second = (x_at(&chart, 7.0), y_at(&chart, 12.5));

    // Fixed click anchors: pointer lifecycle is consumed, activations own semantic placement.
    assert!(chart.set_drawing_tool(
        Some(DrawingKind::TrendLine),
        Some(r##"{"color":"#ff0000"}"##),
        None,
    ));
    let down = chart.drawing_tool_pointer_down(first.0, first.1, DrawingModifiers::default());
    assert!(down.consumed);
    assert_eq!(down.created, None);
    assert!(!down.pointer_capture);
    let first_click = chart.drawing_tool_activate(first.0, first.1, DrawingModifiers::default());
    assert!(first_click.consumed && first_click.changed);
    assert_eq!(first_click.created, None);
    chart.drawing_tool_pointer_move(second.0, second.1, DrawingModifiers::default(), false);
    assert!(chart.drawing_create_active());
    let second_click = chart.drawing_tool_activate(second.0, second.1, DrawingModifiers::default());
    let trend = second_click.created.expect("second fixed anchor commits");
    assert_eq!(chart.active_drawing_tool(), None);
    assert_eq!(chart.drawing(trend).unwrap().kind, DrawingKind::TrendLine);

    // Press placement is tool metadata, not a host special case.
    assert!(chart.set_drawing_tool(Some(DrawingKind::Text), None, None));
    let text = chart.drawing_tool_pointer_down(first.0, first.1, DrawingModifiers::default());
    let text_id = text.created.expect("press-anchored text commits on press");
    assert!(text.request_text_edit);
    assert!(chart.drawing_requests_text_edit(text_id));
    assert_eq!(chart.active_drawing_tool(), None);

    // Freehand owns a captured pointer stream and commits on release.
    assert!(chart.set_drawing_tool(Some(DrawingKind::Brush), None, None));
    let brush_down = chart.drawing_tool_pointer_down(first.0, first.1, DrawingModifiers::default());
    assert!(brush_down.pointer_capture);
    assert!(chart.drawing_tool_capture_active());
    assert!(
        chart
            .drawing_tool_pointer_move(second.0, second.1, DrawingModifiers::default(), true)
            .changed
    );
    let release = (x_at(&chart, 8.0), y_at(&chart, 11.5));
    let brush_up = chart.drawing_tool_pointer_up(release.0, release.1, DrawingModifiers::default());
    let brush = brush_up.created.expect("freehand commits on release");
    let brush_drawing = chart.drawing(brush).unwrap();
    assert_eq!(brush_drawing.kind, DrawingKind::Brush);
    let final_point = brush_drawing.points.last().unwrap();
    assert!((final_point.logical - 8.0).abs() < 1e-6);
    assert!((final_point.price - 11.5).abs() < 1e-6);
    assert_eq!(chart.active_drawing_tool(), None);

    // Variable sequences use the same activation route and an explicit generic finish action.
    assert!(chart.set_drawing_tool(Some(DrawingKind::Path), None, None));
    assert!(chart.drawing_tool_sequence_active());
    assert_eq!(
        chart
            .drawing_tool_activate(first.0, first.1, DrawingModifiers::default())
            .created,
        None
    );
    assert_eq!(
        chart
            .drawing_tool_activate(second.0, second.1, DrawingModifiers::default())
            .created,
        None
    );
    let path = chart
        .drawing_tool_finish()
        .created
        .expect("valid variable sequence commits on finish");
    assert_eq!(chart.drawing(path).unwrap().kind, DrawingKind::Path);
    assert_eq!(chart.active_drawing_tool(), None);
}

#[test]
fn drawing_tool_controller_enforces_pane_constraint_and_preserves_arm_on_capture_cancel() {
    let mut chart = settled_chart();
    chart.add_pane(true);
    chart.build_frame();
    let first_pane_y = chart.panes[0].top + chart.panes[0].height * 0.5;
    let second_pane_y = chart.panes[1].top + chart.panes[1].height * 0.5;

    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, Some(0)));
    let rejected = chart.drawing_tool_activate(100.0, second_pane_y, DrawingModifiers::default());
    assert!(rejected.consumed);
    assert_eq!(rejected.created, None);
    assert!(!chart.drawing_create_active());
    let accepted = chart.drawing_tool_activate(100.0, first_pane_y, DrawingModifiers::default());
    assert!(accepted.changed);
    assert!(chart.drawing_create_active());
    assert_eq!(chart.pending_drawing().unwrap().drawing.pane_index, 0);

    chart.cancel_drawing_creation();
    assert!(!chart.drawing_create_active());
    assert_eq!(chart.active_drawing_tool(), Some(DrawingKind::TrendLine));
    assert_eq!(chart.active_drawing_tool_pane(), Some(0));

    chart.cancel_drawing_tool();
    assert_eq!(chart.active_drawing_tool(), None);
}

#[test]
fn path_creation_requires_finish_and_supports_pop_and_history() {
    let mut chart = settled_chart();
    assert!(chart.drawing_create_begin(DrawingKind::Path, None));

    let first = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let second = (x_at(&chart, 4.0), y_at(&chart, 12.5));
    let third = (x_at(&chart, 7.0), y_at(&chart, 11.0));
    assert_eq!(
        chart.drawing_create_click(first.0, first.1, DrawingModifiers::default()),
        -1
    );
    assert_eq!(chart.drawing_create_finish(), 0);
    assert!(chart.drawing_create_active());
    assert_eq!(
        chart.drawing_create_click(second.0, second.1, DrawingModifiers::default()),
        -1
    );
    assert_eq!(
        chart.drawing_create_click(second.0, second.1, DrawingModifiers::default()),
        -1
    );
    assert_eq!(chart.pending_drawing().unwrap().drawing.points.len(), 2);
    assert_eq!(
        chart.drawing_create_click(third.0, third.1, DrawingModifiers::default()),
        -1
    );
    assert_eq!(chart.pending_drawing().unwrap().drawing.points.len(), 3);

    assert!(chart.drawing_create_pop_anchor());
    assert_eq!(chart.pending_drawing().unwrap().drawing.points.len(), 2);
    assert_eq!(
        chart.drawing_create_click(third.0, third.1, DrawingModifiers::default()),
        -1
    );

    let id = chart.drawing_create_finish();
    assert!(id > 0);
    assert!(!chart.drawing_create_active());
    assert_eq!(chart.selected_drawing(), Some(id));
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.kind, DrawingKind::Path);
    assert_eq!(drawing.points.len(), 3);
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none());
    assert!(chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().points.len(), 3);
}

#[test]
fn path_uses_straight_segments_and_every_vertex_is_editable() {
    let mut chart = settled_chart();
    let points = vec![
        DrawingPoint {
            logical: 2.0,
            price: 10.5,
        },
        DrawingPoint {
            logical: 4.0,
            price: 12.5,
        },
        DrawingPoint {
            logical: 7.0,
            price: 11.0,
        },
    ];
    let id = chart
        .add_drawing(DrawingKind::Path, 0, points.clone(), None)
        .unwrap();
    let frame = chart.build_frame();
    assert_eq!(
        frame.panes[0]
            .main
            .iter()
            .filter(|primitive| matches!(
                primitive,
                Prim::Polyline {
                    point_count: 3,
                    line_type: LineType::Simple,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(!frame.panes[0]
        .main
        .iter()
        .any(|primitive| matches!(primitive, Prim::Triangle { .. })));
    let path_px = chart.drawing_px(chart.drawing(id).unwrap()).unwrap();
    let arrow = path_arrow_points(&path_px, chart.drawing(id).unwrap().width, 1.0).unwrap();
    let wing_midpoint = (
        (arrow[0].0 + arrow[1].0) / 2.0,
        (arrow[0].1 + arrow[1].1) / 2.0,
    );
    assert_eq!(
        chart
            .hit_test_drawing(wing_midpoint.0, wing_midpoint.1)
            .unwrap()
            .part,
        DrawingDragPart::Body
    );

    chart.set_selected_drawing(Some(id));
    let middle = (
        x_at(&chart, points[1].logical),
        y_at(&chart, points[1].price),
    );
    assert_eq!(
        chart.hit_test_drawing(middle.0, middle.1),
        Some(DrawingHit {
            id,
            part: DrawingDragPart::Anchor(1),
            cursor: "pointer"
        })
    );
    assert!(chart.drawing_drag_start_at(middle.0, middle.1));
    chart.drawing_drag_to(
        middle.0 + 20.0,
        middle.1 + 10.0,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert_ne!(chart.drawing(id).unwrap().points[1], points[1]);
    assert_eq!(chart.drawing(id).unwrap().points[0], points[0]);
    assert_eq!(chart.drawing(id).unwrap().points[2], points[2]);

    chart.set_selected_drawing(None);
    let updated = chart.drawing(id).unwrap().points.clone();
    let path_px = chart.drawing_px(chart.drawing(id).unwrap()).unwrap();
    let first_px = path_px[0];
    let second_px = path_px[1];
    let segment_mid = (
        (first_px.0 + second_px.0) / 2.0,
        (first_px.1 + second_px.1) / 2.0,
    );
    assert_eq!(
        chart
            .hit_test_drawing(segment_mid.0, segment_mid.1)
            .unwrap()
            .part,
        DrawingDragPart::Body
    );
    assert!(chart.drawing_drag_start_at(segment_mid.0, segment_mid.1));
    chart.drawing_drag_to(
        segment_mid.0 + 15.0,
        segment_mid.1,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    assert!(chart
        .drawing(id)
        .unwrap()
        .points
        .iter()
        .zip(updated)
        .all(|(after, before)| after.logical != before.logical));
}

#[test]
fn official_rectangle_preview_commit_and_axis_views_are_engine_owned() {
    let mut chart = settled_chart();
    let primary = Color::rgb(
        DEFAULT_PRIMARY_RGB.0,
        DEFAULT_PRIMARY_RGB.1,
        DEFAULT_PRIMARY_RGB.2,
    );
    chart.axis_w = 80.0;
    chart.pane_w = 720.0;
    chart.time_scale.set_width(720.0);
    chart.fit_content();
    chart.build_frame();
    let options = r##"{
        "fill_color":"rgba(200,50,100,0.75)",
        "preview_fill_color":"rgba(200,50,100,0.25)",
        "border_visible":false,
        "show_labels":true,
        "axis_bands_visible":true,
        "label_color":"rgba(200,50,100,1)",
        "label_text_color":"#ffffff",
        "snap_time_to_data":true
    }"##;
    assert!(chart.drawing_create_begin(DrawingKind::Rectangle, Some(options)));
    let spacing = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    let first_x = x_at(&chart, 2.0) + spacing * 0.36;
    assert_eq!(
        chart.drawing_create_click(first_x, y_at(&chart, 10.25), DrawingModifiers::default()),
        -1
    );
    chart.drawing_create_move(
        x_at(&chart, 6.0) + spacing * 0.41,
        y_at(&chart, 12.25),
        DrawingModifiers::default(),
    );
    let pending = chart.pending_drawing().unwrap();
    assert_eq!(pending.drawing.points[0].logical, 2.0);
    assert_eq!(pending.preview.unwrap().logical, 6.0);

    let preview = chart.build_frame();
    assert!(preview.panes[0].main.iter().any(|primitive| {
        matches!(primitive, Prim::Rect { color, .. }
            if *color == Color::rgba(200, 50, 100, 64))
    }));
    assert!(!preview.panes[0].main.iter().any(|primitive| {
        matches!(primitive, Prim::RectFrame { color, .. }
            if *color == Color::rgb(200, 50, 100))
    }));
    let preview_axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert_eq!(preview_axis.bands.len(), 2);
    assert!(preview_axis.bands.iter().any(|band| band.y < chart.pane_h
        && band.color == Color::rgba(primary.r(), primary.g(), primary.b(), 64)));
    assert!(preview_axis
        .bands
        .iter()
        .any(|band| band.y >= chart.pane_h && band.color == Color::rgba(200, 50, 100, 32)));
    assert_eq!(
        preview_axis
            .labels
            .iter()
            .filter(|label| {
                label.color == Color::rgb(255, 255, 255)
                    && matches!(label.background, Some((.., color)) if color == Color::rgb(200, 50, 100))
            })
            .count(),
        2
    );
    assert_eq!(
        preview_axis
            .labels
            .iter()
            .filter(|label| matches!(label.background, Some((.., color)) if color == primary))
            .count(),
        2
    );

    let id = chart.drawing_create_click(
        x_at(&chart, 6.0) + spacing * 0.41,
        y_at(&chart, 12.25),
        DrawingModifiers::default(),
    );
    assert!(id > 0);
    let drawing = chart.drawing(id as DrawingId).unwrap();
    assert_eq!(drawing.points[0].logical, 2.0);
    assert_eq!(drawing.points[1].logical, 6.0);
    assert!(!drawing.border_visible);

    let committed = chart.build_frame();
    assert!(committed.panes[0].main.iter().any(|primitive| {
        matches!(primitive, Prim::Rect { color, .. }
            if *color == Color::rgba(200, 50, 100, 191))
    }));
    chart.set_selected_drawing(None);
    let committed_axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert_eq!(committed_axis.bands.len(), 2);
    assert!(committed_axis
        .bands
        .iter()
        .all(|band| band.color == Color::rgba(200, 50, 100, 96)));
    let mut axis_primitives = Vec::new();
    chart.build_axis_primitives_into(&committed_axis, &mut axis_primitives);
    assert!(axis_primitives.iter().any(|primitive| {
        matches!(primitive, Prim::Rect { color, .. }
            if *color == Color::rgba(200, 50, 100, 96))
    }));
    assert!(committed_axis
        .labels
        .iter()
        .any(|label| label.text == "10.25"));
    assert!(committed_axis
        .labels
        .iter()
        .any(|label| label.text == "12.25"));

    let applied = chart
        .series_apply_price_format_json(0, r#"{"type":"price","precision":4,"min_move":0.0001}"#);
    assert!(applied);
    let reformatted_axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert!(reformatted_axis
        .labels
        .iter()
        .any(|label| label.text == "10.2500"));
    assert!(reformatted_axis
        .labels
        .iter()
        .any(|label| label.text == "12.2500"));
    assert_eq!(
        committed_axis
            .labels
            .iter()
            .filter(|label| label.text == "1/1/1970")
            .count(),
        2
    );
}

#[test]
fn selected_rectangle_owns_live_price_scale_territory_by_default() {
    let mut chart = settled_chart();
    chart.axis_w = 80.0;
    chart.pane_w = 720.0;
    chart.time_scale.set_width(720.0);
    chart.fit_content();
    chart.build_frame();
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.25,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.25,
                },
            ],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(None);

    let unselected = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert!(unselected.bands.is_empty());
    assert!(!unselected.labels.iter().any(|label| {
        label.background.is_some() && matches!(label.text.as_str(), "10.25" | "12.25")
    }));

    chart.set_selected_drawing(Some(id));
    let selected = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    let primary = Color::rgb(
        DEFAULT_PRIMARY_RGB.0,
        DEFAULT_PRIMARY_RGB.1,
        DEFAULT_PRIMARY_RGB.2,
    );
    assert_eq!(
        selected.bands.len(),
        1,
        "selection adds price territory only"
    );
    assert_eq!(
        selected.bands[0].color,
        Color::rgba(primary.r(), primary.g(), primary.b(), 64)
    );
    assert!(
        selected
            .labels
            .iter()
            .filter(|label| {
                label.background.is_some() && matches!(label.text.as_str(), "10.25" | "12.25")
            })
            .count()
            >= 2
    );
    assert!(selected
        .labels
        .iter()
        .filter(|label| {
            label.background.is_some() && matches!(label.text.as_str(), "10.25" | "12.25")
        })
        .all(|label| label.background.unwrap().4 == primary));
    assert!(!selected
        .labels
        .iter()
        .any(|label| { label.background.is_some() && label.text.contains('/') }));
    let original_price_band = selected
        .bands
        .iter()
        .find(|band| band.y < chart.pane_h)
        .copied()
        .unwrap();

    let first = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!(chart.drawing_drag_start_at(first.0, first.1));
    chart.drawing_drag_to(first.0, first.1 + 20.0, DrawingModifiers::default());
    let dragging = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    let dragged_price_band = dragging
        .bands
        .iter()
        .find(|band| band.y < chart.pane_h)
        .copied()
        .unwrap();
    assert_ne!(dragged_price_band, original_price_band);

    chart.drawing_drag_end();
    chart.set_selected_drawing(None);
    let deselected = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert!(deselected.bands.is_empty());

    chart.left_axis_w = 80.0;
    chart
        .apply_options(r#"{"leftPriceScale":{"visible":true}}"#)
        .unwrap();
    chart.set_series_price_scale(0, crate::PriceScaleTarget::Overlay);
    assert!(chart.drawing_apply_options(id, r#"{"price_scale_id":"overlay"}"#));
    chart.set_selected_drawing(Some(id));
    let overlay = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    assert_eq!(
        overlay.bands.len(),
        1,
        "active overlay drawings use the default right axis only"
    );
    assert_eq!(overlay.bands[0].x, chart.pane_left + chart.pane_w);
}

#[test]
fn official_rectangle_uses_its_attached_left_scale_for_geometry_interaction_and_axis_views() {
    let mut chart = settled_chart();
    chart.set_series_price_scale(0, crate::PriceScaleTarget::Left);
    chart
        .apply_options(r#"{"leftPriceScale":{"visible":true},"rightPriceScale":{"visible":false}}"#)
        .unwrap();
    chart.left_axis_w = 80.0;
    chart.pane_left = 80.0;
    chart.pane_w = 720.0;
    chart.time_scale.set_width(720.0);
    chart.fit_content();
    chart.build_frame();

    let first = (x_at(&chart, 2.0), y_at(&chart, 10.25));
    let second = (x_at(&chart, 6.0), y_at(&chart, 12.25));
    assert!(chart.drawing_create_begin(
        DrawingKind::Rectangle,
        Some(
            r##"{"price_scale_id":"left","fill_color":"rgba(200,50,100,0.75)","border_visible":false,"show_labels":true,"axis_bands_visible":true}"##,
        ),
    ));
    assert_eq!(
        chart.drawing_create_click(first.0, first.1, DrawingModifiers::default()),
        -1
    );
    let id = chart.drawing_create_click(second.0, second.1, DrawingModifiers::default());
    assert!(id > 0);
    let id = id as DrawingId;
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.price_scale, DrawingPriceScale::Left);
    assert!((drawing.points[0].price - 10.25).abs() < 1e-9);
    assert!((drawing.points[1].price - 12.25).abs() < 1e-9);
    let converted = chart.drawing_point_to_coordinate(id, 0).unwrap();
    assert!((converted.0 - first.0).abs() < 1e-9);
    assert!((converted.1 - first.1).abs() < 1e-9);

    let midpoint = ((first.0 + second.0) / 2.0, (first.1 + second.1) / 2.0);
    chart.set_selected_drawing(Some(id));
    assert!(chart.drawing_drag_start_at(midpoint.0, midpoint.1));
    chart.drawing_drag_to(midpoint.0, midpoint.1 + 10.0, DrawingModifiers::default());
    chart.drawing_drag_end();
    assert_ne!(chart.drawing(id).unwrap().points[0].price, 10.25);

    let axis = chart.build_axis_frame(
        80.0,
        |text, _bold| text.len() as f64 * 7.0,
        |text, _bold| text.len() as f64 * 6.0,
    );
    let price_band = axis
        .bands
        .iter()
        .find(|band| band.y < chart.pane_h)
        .expect("left price-axis band");
    assert_eq!(price_band.x, chart.pane_left - 15.0);
    assert!(price_band.x + price_band.width <= chart.pane_left);
    assert!(
        axis.labels
            .iter()
            .filter(|label| {
                label.background.is_some()
                    && label.align == crate::AxisTextAlign::Right
                    && label.x < chart.pane_left
            })
            .count()
            >= 2
    );
    assert!(!axis.labels.iter().any(|label| {
        label.background.is_some()
            && label.align == crate::AxisTextAlign::Left
            && label.x > chart.pane_left + chart.pane_w
    }));
}

#[test]
fn live_options_update_pending_drawing_without_losing_anchors() {
    let mut chart = settled_chart();
    assert!(!chart.drawing_create_apply_options(r##"{"width":4.0}"##));
    assert!(chart.drawing_create_begin(
        DrawingKind::TrendLine,
        Some(r##"{"color":"#ff0000","width":2.0}"##),
    ));
    let first = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    assert_eq!(
        chart.drawing_create_click(first.0, first.1, DrawingModifiers::default()),
        -1
    );
    let points_before = chart.pending_drawing().unwrap().drawing.points.clone();
    let preview_before = chart.pending_drawing().unwrap().preview;

    assert!(!chart.drawing_create_apply_options("{"));
    assert!(chart.drawing_create_apply_options(r##"{"width":4.0,"style":"dashed"}"##));
    let pending = chart.pending_drawing().unwrap();
    assert_eq!(pending.drawing.points, points_before);
    assert_eq!(pending.preview, preview_before);
    assert_eq!(pending.drawing.color, "#ff0000");
    assert_eq!(pending.drawing.width, 4.0);
    assert_eq!(pending.drawing.style, LineStyle::Dashed);

    let second = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    let id = chart.drawing_create_click(second.0, second.1, DrawingModifiers::default());
    let drawing = chart.drawing(id as DrawingId).unwrap();
    assert_eq!(drawing.points[0], points_before[0]);
    assert_eq!(drawing.color, "#ff0000");
    assert_eq!(drawing.width, 4.0);
}

#[test]
fn one_anchor_kinds_commit_on_the_first_click() {
    let mut chart = settled_chart();
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    let id = chart.drawing_create_click(300.0, y_at(&chart, 11.0), DrawingModifiers::default());
    assert!(id > 0);
    assert!(!chart.drawing_create_active());
    let drawing = chart.drawing(id as DrawingId).unwrap();
    assert!((drawing.points[0].price - 11.0).abs() < 1e-9);
}

#[test]
fn creation_cancel_drops_the_pending_drawing() {
    let mut chart = settled_chart();
    assert!(chart.drawing_create_begin(DrawingKind::Rectangle, None));
    assert_eq!(
        chart.drawing_create_click(100.0, 100.0, DrawingModifiers::default()),
        -1
    );
    chart.drawing_create_cancel();
    assert!(!chart.drawing_create_active());
    assert_eq!(chart.drawings().len(), 0);
}

#[test]
fn remove_releases_selection_and_clear_empties_the_store() {
    let mut chart = settled_chart();
    let first = add_trend(&mut chart);
    let second = add_trend(&mut chart);
    chart.set_selected_drawing(Some(second));
    assert!(chart.remove_drawing(second));
    assert_eq!(chart.selected_drawing(), None);
    assert!(!chart.remove_drawing(second));
    assert_eq!(chart.drawings().len(), 1);
    chart.set_selected_drawing(Some(first));
    chart.clear_drawings();
    assert_eq!(chart.drawings().len(), 0);
    assert_eq!(chart.selected_drawing(), None);
    // remove_selected_drawing reports whether anything was selected.
    assert!(!chart.remove_selected_drawing());
    let third = add_trend(&mut chart);
    chart.set_selected_drawing(Some(third));
    assert!(chart.remove_selected_drawing());
    assert_eq!(chart.drawings().len(), 0);
}

#[test]
fn select_drawing_at_arbitrates_click_selection() {
    let mut chart = settled_chart();
    let id = add_trend(&mut chart);
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    // A click on the stroke selects; a click on empty space clears.
    assert!(chart.select_drawing_at((x1 + x2) / 2.0, (y1 + y2) / 2.0));
    assert_eq!(chart.selected_drawing(), Some(id));
    assert!(!chart.select_drawing_at(30.0, 30.0));
    assert_eq!(chart.selected_drawing(), None);
}

#[test]
fn drawings_json_lists_in_z_order() {
    let mut chart = settled_chart();
    let first = add_trend(&mut chart);
    let second = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r#"{"text":"note"}"#),
        )
        .unwrap();
    let list: serde_json::Value = serde_json::from_str(&chart.drawings_json()).unwrap();
    let array = list.as_array().unwrap();
    assert_eq!(array.len(), 2);
    assert_eq!(array[0]["id"], first);
    assert_eq!(array[0]["kind"], "trend_line");
    assert_eq!(array[1]["id"], second);
    assert_eq!(array[1]["kind"], "text");
    assert_eq!(array[1]["text"], "note");
    assert_eq!(array[1]["points"][0]["logical"], 5.0);
}

#[test]
fn hits_are_restricted_to_the_pane_under_the_cursor() {
    let mut chart = settled_chart();
    add_trend(&mut chart);
    // Off the pane entirely (negative x handled by the guard; y below the pane bottom).
    assert!(chart.hit_test_drawing(100.0, -5.0).is_none());
    assert!(chart.hit_test_drawing(-3.0, 100.0).is_none());
    assert!(chart.hit_test_drawing(100.0, chart.pane_h + 5.0).is_none());
}

// --- modifier snaps (magnet / straighten) ---

const MAGNET: DrawingModifiers = DrawingModifiers {
    magnet: true,
    straighten: false,
};
const STRAIGHTEN: DrawingModifiers = DrawingModifiers {
    magnet: false,
    straighten: true,
};
const BOTH: DrawingModifiers = DrawingModifiers {
    magnet: true,
    straighten: true,
};
const NONE: DrawingModifiers = DrawingModifiers {
    magnet: false,
    straighten: false,
};

/// A candle chart with DISTINCT OHLC per bar so the magnet's nearest-of-four choice is visible.
fn ohlc_chart() -> ChartEngine {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
    let open = [10.0, 11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0];
    let high = [11.0, 12.0, 13.0, 12.0, 11.0, 12.0, 13.0, 14.0, 13.0, 12.0];
    let low = [9.0, 10.0, 11.0, 10.0, 9.0, 10.0, 11.0, 12.0, 11.0, 10.0];
    let close = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
    chart
        .set_series_data(0, &times, &open, &high, &low, &close)
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();
    chart
}

#[test]
fn magnet_snaps_placement_to_nearest_bar_and_ohlc() {
    let mut chart = ohlc_chart();
    assert!(chart.drawing_create_begin(DrawingKind::TrendLine, None));
    // Click BETWEEN bars 2 and 3. The shared crosshair cell rule keeps the exact boundary on bar 2;
    // its values are {o 12, h 13, l 11, c 11}, and the cursor is nearest its open at 12.
    let x = (x_at(&chart, 2.0) + x_at(&chart, 3.0)) / 2.0;
    let y = y_at(&chart, 12.0) - 1.0; // just above 12 (closer to 12 than to 11 or 13)
    assert_eq!(chart.drawing_create_click(x, y, MAGNET), -1);
    let pending = chart.pending_drawing().unwrap();
    assert_eq!(pending.drawing.points.len(), 1);
    let point = pending.drawing.points[0];
    assert_eq!(point.logical, 2.0, "x uses the crosshair's bar-cell rule");
    assert_eq!(
        point.price, 12.0,
        "price snaps to the nearest of the bar's OHLC"
    );
    // Without the modifier the same click stays raw (fractional logical, unscaled price).
    chart.drawing_create_cancel();
    assert!(chart.drawing_create_begin(DrawingKind::TrendLine, None));
    assert_eq!(chart.drawing_create_click(x, y, NONE), -1);
    let raw = chart.pending_drawing().unwrap().drawing.points[0];
    assert!((raw.logical - 3.0).abs() > 1e-9);
    assert!((raw.price - 12.0).abs() > 1e-9);
}

#[test]
fn drawing_magnet_ignores_derived_indicator_lines() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[10.0, 20.0, 100.0],
            &[10.0, 20.0, 100.0],
            &[10.0, 20.0, 100.0],
            &[10.0, 20.0, 100.0],
        )
        .unwrap();
    let sma = chart.add_sma(0, 2).expect("SMA output");
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();

    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    let indicator_plot = chart.data.plot(sma);
    let indicator_row = indicator_plot
        .search(1, MismatchDirection::None)
        .expect("SMA row at logical index 1");
    let indicator_value = indicator_plot.value_at(indicator_row, PlotValueIndex::Close);
    assert_eq!(indicator_value, 15.0);

    let id = chart.drawing_create_click(x_at(&chart, 1.0), y_at(&chart, indicator_value), MAGNET);
    assert!(id > 0);
    assert_eq!(
        chart.drawing(id as DrawingId).unwrap().points[0].price,
        20.0,
        "the derived SMA at 15 must not attract a drawing intended for the source series"
    );
}

#[test]
fn path_magnet_snaps_every_placed_vertex() {
    let mut chart = ohlc_chart();
    assert!(chart.drawing_create_begin(DrawingKind::Path, None));
    assert_eq!(
        chart.drawing_create_click(x_at(&chart, 2.0), y_at(&chart, 12.9), MAGNET),
        -1
    );
    assert_eq!(
        chart.drawing_create_click(x_at(&chart, 7.0), y_at(&chart, 12.1), MAGNET),
        -1
    );
    let id = chart.drawing_create_finish();
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(
        drawing.points[0],
        DrawingPoint {
            logical: 2.0,
            price: 13.0
        }
    );
    assert_eq!(
        drawing.points[1],
        DrawingPoint {
            logical: 7.0,
            price: 12.0
        }
    );
}

#[test]
fn magnet_off_the_data_keeps_the_raw_point() {
    let mut chart = ohlc_chart();
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    // Far right of the last bar (logical ~40): no bar to snap to, so the click stands.
    let x = x_at(&chart, 9.0) + 31.0 * 80.0;
    let y = y_at(&chart, 10.75);
    let id = chart.drawing_create_click(x, y, MAGNET);
    assert!(id > 0);
    let point = chart.drawing(id as DrawingId).unwrap().points[0];
    assert!((point.price - 10.75).abs() < 1e-6);
}

#[test]
fn horizontal_line_magnet_uses_the_live_pointer_bar_during_creation_and_editing() {
    let mut chart = ohlc_chart();
    let x7 = x_at(&chart, 7.0);
    let y14 = y_at(&chart, 14.0);

    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    chart.drawing_create_move(x7, y14 + 0.5, MAGNET);
    let preview = chart.pending_drawing().unwrap().preview.unwrap();
    assert_eq!(preview.logical, 7.0);
    assert_eq!(preview.price, 14.0);
    let id = chart.drawing_create_click(x7, y14 + 0.5, MAGNET) as DrawingId;
    assert_ne!(id, 0);
    assert_eq!(chart.drawing(id).unwrap().points[0].price, 14.0);

    // The line spans the pane, but the pointer X still chooses the candle used by the magnet.
    let x2 = x_at(&chart, 2.0);
    let y13 = y_at(&chart, 13.0);
    chart.set_selected_drawing(Some(id));
    assert!(chart.drawing_drag_start_at(x2, y14));
    chart.drawing_drag_to(x2, y13 + 0.5, MAGNET);
    chart.drawing_drag_end();
    let point = chart.drawing(id).unwrap().points[0];
    assert_eq!(
        point.price, 13.0,
        "body drag must snap against the bar under the pointer, not the stored anchor X"
    );
    assert_eq!(
        point.logical, 7.0,
        "the full-width line's stored X is semantic-free"
    );

    // Dragging the selection handle across bars has the same pointer-X semantics.
    assert!(chart.drawing_drag_start_at(x7, y13));
    chart.drawing_drag_to(x_at(&chart, 4.0), y_at(&chart, 9.0) - 0.5, MAGNET);
    chart.drawing_drag_end();
    let point = chart.drawing(id).unwrap().points[0];
    assert_eq!(
        point.price, 9.0,
        "anchor drag must resolve the live pointer bar before freezing the unused logical value"
    );
    assert_eq!(
        point.logical, 7.0,
        "anchor edits must preserve the unused stored X"
    );

    // Horizontal rays use the same live-pointer resolver, but retain their meaningful start X.
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalRay, None));
    chart.drawing_create_move(x2, y13 + 0.5, MAGNET);
    let preview = chart.pending_drawing().unwrap().preview.unwrap();
    assert_eq!(
        preview,
        DrawingPoint {
            logical: 2.0,
            price: 13.0
        }
    );
    let ray = chart.drawing_create_click(x2, y13 + 0.5, MAGNET) as DrawingId;
    assert_eq!(
        chart.drawing(ray).unwrap().points[0],
        DrawingPoint {
            logical: 2.0,
            price: 13.0
        }
    );

    // Without Ctrl, Horizontal Line placement keeps the raw pointer price.
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    let raw_y = y_at(&chart, 12.4);
    let raw = chart.drawing_create_click(x2, raw_y, NONE) as DrawingId;
    assert!((chart.drawing(raw).unwrap().points[0].price - 12.4).abs() < 1e-6);
}

#[test]
fn straighten_snaps_a_trend_anchor_drag_to_0_45_90() {
    let mut chart = ohlc_chart();
    let id = add_trend(&mut chart); // (2, 10.5) -> (7, 12.5)
    chart.set_selected_drawing(Some(id));
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    // Grab anchor 1 and drag it ALMOST horizontally (slight downward slope): straighten to 0°.
    assert!(chart.drawing_drag_start_at(x2, y2));
    chart.drawing_drag_to(x2 + 60.0, y1 + 6.0, STRAIGHTEN);
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!(
        (points[0].price - points[1].price).abs() < 1e-6,
        "near-horizontal drag straightens to horizontal: {points:?}"
    );
    // Now drag anchor 1 into a steep ~90° direction above anchor 0: straighten to vertical.
    // (Re-grab at the anchor's CURRENT position — the previous drag moved it.)
    let current = chart.drawing(id).unwrap().points[1];
    let (gx, gy) = chart.drawing_to_px(0, current).unwrap();
    assert!(chart.drawing_drag_start_at(gx, gy));
    chart.drawing_drag_to(x1 + 4.0, y1 - 120.0, STRAIGHTEN);
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!(
        (points[0].logical - points[1].logical).abs() < 1e-6,
        "near-vertical drag straightens to vertical: {points:?}"
    );
    // 45°: drag so the pixel run is roughly (but not exactly) diagonal.
    let spacing = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    let current = chart.drawing(id).unwrap().points[1];
    let (gx, gy) = chart.drawing_to_px(0, current).unwrap();
    assert!(chart.drawing_drag_start_at(gx, gy));
    chart.drawing_drag_to(x1 + 4.0 * spacing, y1 - 3.6 * spacing, STRAIGHTEN);
    chart.drawing_drag_end();
    let drawing = chart.drawing(id).unwrap();
    let (ax, ay) = chart.drawing_to_px(0, drawing.points[0]).unwrap();
    let (bx, by) = chart.drawing_to_px(0, drawing.points[1]).unwrap();
    assert!(
        ((bx - ax).abs() - (by - ay).abs()).abs() < 1.0,
        "diagonal drag straightens to 45°: run ({}, {})",
        bx - ax,
        by - ay
    );
    // Toggling the modifier OFF mid-session recomputes from the start snapshot (live response).
    let id2 = add_trend(&mut chart);
    chart.set_selected_drawing(Some(id2));
    assert!(chart.drawing_drag_start_at(x2, y2));
    chart.drawing_drag_to(x2 + 60.0, y1 + 6.0, STRAIGHTEN);
    chart.drawing_drag_to(x2 + 30.0, y2 - 45.0, NONE);
    chart.drawing_drag_end();
    let points = &chart.drawing(id2).unwrap().points;
    assert!(
        (points[0].price - points[1].price).abs() > 1e-6,
        "released: free again"
    );
}

#[test]
fn straighten_constrains_a_body_drag_to_the_dominant_axis() {
    let mut chart = ohlc_chart();
    let id = add_trend(&mut chart);
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    let (x2, y2) = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    // Mostly-horizontal body drag: the price must not change.
    assert!(chart.drawing_drag_start_at(mx, my));
    chart.drawing_drag_to(mx + 80.0, my + 10.0, STRAIGHTEN);
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!((points[0].logical - 2.0).abs() > 0.5, "moved horizontally");
    assert!((points[0].price - 10.5).abs() < 1e-9, "price frozen");
    assert!((points[1].price - 12.5).abs() < 1e-9, "price frozen");
    // Mostly-vertical body drag: the logicals must not change. (Re-grab at the segment's
    // CURRENT midpoint — the previous drag moved it.)
    let drawing = chart.drawing(id).unwrap();
    let (ax, ay) = chart.drawing_to_px(0, drawing.points[0]).unwrap();
    let (bx, by) = chart.drawing_to_px(0, drawing.points[1]).unwrap();
    let (cx, cy) = ((ax + bx) / 2.0, (ay + by) / 2.0);
    let logical_before = drawing.points[0].logical;
    assert!(chart.drawing_drag_start_at(cx, cy));
    chart.drawing_drag_to(cx + 8.0, cy - 70.0, STRAIGHTEN);
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert!(
        (points[0].logical - logical_before).abs() < 1e-6,
        "logical frozen"
    );
    assert!(
        (points[1].logical - (logical_before + 5.0)).abs() < 1e-6,
        "logical frozen"
    );
}

#[test]
fn straighten_squares_a_rectangle_on_placement_and_drag() {
    let mut chart = ohlc_chart();
    // Placement: the second click with straighten snaps the corner to a square. Keep the drag
    // small enough that the squared corner stays on-pane.
    assert!(chart.drawing_create_begin(DrawingKind::Rectangle, None));
    let (x1, y1) = (x_at(&chart, 2.0), y_at(&chart, 10.0));
    assert_eq!(chart.drawing_create_click(x1, y1, NONE), -1);
    let spacing = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    let id = chart.drawing_create_click(x1 + 2.0 * spacing, y1 - 50.0, BOTH);
    assert!(id > 0);
    let drawing = chart.drawing(id as DrawingId).unwrap();
    let (ax, ay) = chart.drawing_to_px(0, drawing.points[0]).unwrap();
    let (bx, by) = chart.drawing_to_px(0, drawing.points[1]).unwrap();
    assert!(
        ((bx - ax).abs() - (by - ay).abs()).abs() < 1.0,
        "placement straightens to a square: ({}, {})",
        bx - ax,
        by - ay
    );
    // Corner drag with straighten keeps it square.
    chart.set_selected_drawing(Some(id as DrawingId));
    assert!(chart.drawing_drag_start_at(bx, by));
    chart.drawing_drag_to(bx + 60.0, by + 15.0, STRAIGHTEN);
    chart.drawing_drag_end();
    let drawing = chart.drawing(id as DrawingId).unwrap();
    let (cx, cy) = chart.drawing_to_px(0, drawing.points[0]).unwrap();
    let (ddx, ddy) = chart.drawing_to_px(0, drawing.points[1]).unwrap();
    assert!(
        ((ddx - cx).abs() - (ddy - cy).abs()).abs() < 1.0,
        "corner drag keeps the square: ({}, {})",
        ddx - cx,
        ddy - cy
    );
}

/// The crosshair's horizontal line y in the frame (its color is the default crosshair gray).
fn crosshair_hline_y(chart: &mut ChartEngine) -> Option<i32> {
    let frame = chart.build_frame();
    frame.panes[0].main.iter().find_map(|prim| match prim {
        Prim::HLine { y, color, .. }
            if *color
                == Color::rgb(
                    aeris_charts_core::style::DEFAULT_CROSSHAIR_RGB.0,
                    aeris_charts_core::style::DEFAULT_CROSSHAIR_RGB.1,
                    aeris_charts_core::style::DEFAULT_CROSSHAIR_RGB.2,
                ) =>
        {
            Some(*y)
        }
        _ => None,
    })
}

#[test]
fn ctrl_magnet_snaps_the_crosshair_to_ohlc() {
    let mut chart = ohlc_chart();
    // Cursor on bar 3 ({o 11, h 12, l 10, c 11}) at y 11.8 — nearest its high (12) in px.
    let x = x_at(&chart, 3.0);
    let y_free = y_at(&chart, 11.8);
    chart.crosshair = Some((x, y_free));
    // Normal mode without the flag: the horizontal line follows the raw cursor y.
    assert_eq!(crosshair_hline_y(&mut chart), Some(y_free.round() as i32));
    // A host can forward Ctrl while browsing. Without drawing work it stays at the raw price.
    chart.crosshair_ohlc_magnet = true;
    assert_eq!(crosshair_hline_y(&mut chart), Some(y_free.round() as i32));
    // Drawing creation: Ctrl now resolves the same OHLC candidate as the anchor.
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    let (from, to) = chart.visible_range_for_frame().unwrap();
    let snapped_y = chart.crosshair_snap(0, x, y_free, from, to).1;
    assert_eq!(snapped_y.round() as i32, y_at(&chart, 12.0).round() as i32);
    assert_eq!(crosshair_hline_y(&mut chart), None);
    let drawing_id = chart.drawing_create_click(x, y_free, MAGNET) as DrawingId;
    let (_, drawing_y) = chart
        .drawing_to_px(0, chart.drawing(drawing_id).unwrap().points[0])
        .unwrap();
    assert_eq!(
        drawing_y.round() as i32,
        snapped_y.round() as i32,
        "crosshair and drawing magnets must resolve the same pixel-space OHLC candidate"
    );
    // Creation finished: free browsing is raw even if the modifier flag remains set.
    assert_eq!(crosshair_hline_y(&mut chart), Some(y_free.round() as i32));
    chart.crosshair_ohlc_magnet = false;
    // Hidden mode stays hidden even with the flag set.
    chart.crosshair_ohlc_magnet = true;
    chart.crosshair_mode = aeris_charts_core::model::magnet::CrosshairMode::Hidden;
    assert_eq!(crosshair_hline_y(&mut chart), None);
}

#[test]
fn ctrl_crosshair_magnet_ignores_external_study_on_price_pane() {
    let mut chart = ohlc_chart();
    let times = (0..10)
        .map(|index| i64::from(index) * 3_600_000_000_000)
        .collect::<Vec<_>>();
    let values = [Some(11.8); 10];
    chart
        .install_external_study_output(
            7,
            0,
            ExternalStudyOutputDescriptor {
                title: "EMA",
                legend_label: None,
                plot: ExternalStudyPlotKind::Line,
                pane: ExternalStudyPaneTarget::Price,
                scale: ExternalStudyScaleTarget::Primary,
                settings_available: true,
                threshold_region: None,
                point_style: ExternalStudyPointStyle::Uniform,
                input_requirements: ExternalStudyInputRequirements::BARS,
            },
            1,
            &times,
            &values,
        )
        .unwrap();
    chart.build_frame();

    let x = x_at(&chart, 3.0);
    chart.crosshair = Some((x, y_at(&chart, 11.8)));
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    chart.crosshair_ohlc_magnet = true;
    let (from, to) = chart.visible_range_for_frame().unwrap();
    assert_eq!(
        chart
            .crosshair_snap(0, x, y_at(&chart, 11.8), from, to)
            .1
            .round() as i32,
        y_at(&chart, 12.0).round() as i32,
        "the external EMA must remain inspectable without attracting Ctrl magnetism"
    );
}

#[test]
fn ohlc_magnet_snaps_scalar_series_to_the_rendered_value() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    chart.convert_series_kind(0, SeriesKind::Area);
    chart
        .set_series_data(
            0,
            &[1.0, 2.0, 3.0],
            &[11.0, 22.0, 31.0],
            &[12.0, 25.0, 32.0],
            &[9.0, 15.0, 29.0],
            &[10.0, 20.0, 30.0],
        )
        .unwrap();
    chart.time_scale.set_width(800.0);
    chart.fit_content();
    chart.build_frame();

    let x = x_at(&chart, 1.0);
    let empty_y = y_at(&chart, 25.0);
    let rendered_y = y_at(&chart, 20.0);
    chart.crosshair = Some((x, empty_y));
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    chart.crosshair_ohlc_magnet = true;
    let (from, to) = chart.visible_range_for_frame().unwrap();
    assert_eq!(
        chart.crosshair_snap(0, x, empty_y, from, to).1.round() as i32,
        rendered_y.round() as i32,
        "an area series exposes only its rendered close/value to the OHLC magnet"
    );

    let drawing_id = chart.drawing_create_click(x, empty_y, MAGNET) as DrawingId;
    assert!((chart.drawing(drawing_id).unwrap().points[0].price - 20.0).abs() < 1e-9);
}

// --- brush (freehand) ---

fn add_brush(chart: &mut ChartEngine) -> DrawingId {
    chart
        .add_drawing(
            DrawingKind::Brush,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
                DrawingPoint {
                    logical: 4.0,
                    price: 12.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 10.0,
                },
            ],
            None,
        )
        .unwrap()
}

#[test]
fn brush_validates_a_minimum_of_two_points() {
    let mut chart = settled_chart();
    assert!(chart
        .add_drawing(
            DrawingKind::Brush,
            0,
            vec![DrawingPoint {
                logical: 1.0,
                price: 11.0
            }],
            None,
        )
        .is_none());
    assert!(add_brush(&mut chart) > 0);
    assert_eq!(chart.drawings().len(), 1);
}

#[test]
fn brush_body_hit_follows_the_stroke_and_misses_off_path() {
    let mut chart = settled_chart();
    let id = add_brush(&mut chart);
    // Hits at every vertex and along the segments, misses far away.
    for (logical, price) in [(2.0, 10.0), (4.0, 12.0), (6.0, 10.0)] {
        let hit = chart
            .hit_test_drawing(x_at(&chart, logical), y_at(&chart, price))
            .unwrap();
        assert_eq!(hit.id, id);
        assert_eq!(hit.part, DrawingDragPart::Body);
    }
    // Just off a vertex along the stroke still hits; a clear miss off the path does not.
    assert!(chart
        .hit_test_drawing(x_at(&chart, 2.0) + 2.0, y_at(&chart, 10.0))
        .is_some());
    // The straight CHORD midpoint between (2,10) and (4,12) is a MISS beyond tolerance: the
    // smooth curve bends away from the jagged chord — the geometric proof the stroke is drawn
    // (and hit-tested) as a curve, not as jagged segments.
    assert!(
        chart
            .hit_test_drawing(x_at(&chart, 3.0), y_at(&chart, 11.0))
            .is_none(),
        "the smooth curve deviates from the chord midpoint"
    );
    assert!(chart
        .hit_test_drawing(x_at(&chart, 3.0), y_at(&chart, 8.0))
        .is_none());
}

#[test]
fn brush_anchors_live_at_the_two_ends_only() {
    let mut chart = settled_chart();
    let id = add_brush(&mut chart);
    chart.set_selected_drawing(Some(id));
    // The two ends hit as Anchor(0) / Anchor(n-1)...
    let first = chart
        .hit_test_drawing(x_at(&chart, 2.0), y_at(&chart, 10.0))
        .unwrap();
    assert_eq!(first.part, DrawingDragPart::Anchor(0));
    let last = chart
        .hit_test_drawing(x_at(&chart, 6.0), y_at(&chart, 10.0))
        .unwrap();
    assert_eq!(last.part, DrawingDragPart::Anchor(2));
    // ...but the middle vertex hits as Body, not an anchor.
    let middle = chart
        .hit_test_drawing(x_at(&chart, 4.0), y_at(&chart, 12.0))
        .unwrap();
    assert_eq!(middle.part, DrawingDragPart::Body);
}

#[test]
fn brush_end_anchor_drag_moves_only_that_endpoint() {
    let mut chart = settled_chart();
    let id = add_brush(&mut chart);
    chart.set_selected_drawing(Some(id));
    assert!(chart.drawing_drag_start_at(x_at(&chart, 6.0), y_at(&chart, 10.0)));
    chart.drawing_drag_to(x_at(&chart, 8.0), y_at(&chart, 12.5), NONE);
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    assert_eq!(
        points[0],
        DrawingPoint {
            logical: 2.0,
            price: 10.0
        }
    );
    assert_eq!(
        points[1],
        DrawingPoint {
            logical: 4.0,
            price: 12.0
        }
    );
    assert!((points[2].logical - 8.0).abs() < 1e-6);
    assert!((points[2].price - 12.5).abs() < 1e-6);
}

#[test]
fn brush_body_drag_translates_every_point() {
    let mut chart = settled_chart();
    let id = add_brush(&mut chart);
    // Grab the stroke's middle (a body hit).
    assert!(chart.drawing_drag_start_at(x_at(&chart, 4.0), y_at(&chart, 12.0)));
    let y_unit = y_at(&chart, 11.0) - y_at(&chart, 12.0);
    let dx = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    chart.drawing_drag_to(x_at(&chart, 4.0) + dx, y_at(&chart, 12.0) + y_unit, NONE);
    chart.drawing_drag_end();
    let points = &chart.drawing(id).unwrap().points;
    for (index, (logical, price)) in [(2.0, 10.0), (4.0, 12.0), (6.0, 10.0)].iter().enumerate() {
        assert!((points[index].logical - (logical + 1.0)).abs() < 1e-6);
        assert!((points[index].price - (price - 1.0)).abs() < 1e-6);
    }
}

#[test]
fn brush_renders_as_one_smooth_curved_polyline() {
    let mut chart = settled_chart();
    add_brush(&mut chart);
    let frame = chart.build_frame();
    let polylines: Vec<_> = frame.panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Polyline {
                first_point,
                point_count,
                line_type,
                ..
            } => Some((*first_point, *point_count, line_type)),
            _ => None,
        })
        .collect();
    assert_eq!(polylines.len(), 1);
    let (first_point, point_count, line_type) = polylines[0];
    assert_eq!(point_count, 3);
    assert_eq!(
        *line_type,
        aeris_charts_render::draw_list::LineType::Curved,
        "the brush stroke renders as a smooth curve, not jagged segments"
    );
    assert_eq!(first_point as usize, 0);
}

#[test]
fn brush_capture_decimates_input_and_commits_the_live_path() {
    let mut chart = settled_chart();
    assert!(!chart.brush_create_active());
    let start = (x_at(&chart, 2.0), y_at(&chart, 10.0));
    assert!(chart.brush_create_start(Some(r##"{"color":"#123456"}"##), start.0, start.1));
    assert!(chart.brush_create_active());
    // Sub-threshold jitter is decimated (bar spacing is ~80 px here, so 0.5px is noise).
    chart.brush_create_add(start.0 + 0.5, start.1 + 0.5);
    assert_eq!(chart.brush_capture().unwrap().points.len(), 1);
    // A straight drag across four bars with one real corner.
    let spacing = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    for step in 1..=8 {
        chart.brush_create_add(start.0 + step as f64 * spacing * 0.5, start.1);
    }
    chart.brush_create_add(start.0 + 4.5 * spacing, start.1 - 3.0 * spacing);
    chart.brush_create_add(start.0 + 5.0 * spacing, start.1 - 6.0 * spacing);
    // The capture is what the live stroke painted — the commit must not re-shape it.
    let captured: Vec<(f64, f64)> = chart
        .brush_capture()
        .unwrap()
        .points
        .iter()
        .map(|&point| {
            (
                chart.logical_to_coordinate(point.logical).unwrap(),
                chart.series_price_to_coordinate(0, point.price).unwrap(),
            )
        })
        .collect();
    let id = chart.brush_create_end();
    assert!(id > 0);
    assert!(!chart.brush_create_active());
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.kind, DrawingKind::Brush);
    assert_eq!(drawing.color, "#123456");
    let points = &drawing.points;
    // Every decimated sample survives: the committed path is the live stroke's curve.
    assert!(
        points.len() >= captured.len(),
        "no commit-time thinning: {points:?}"
    );
    assert!(points.len() >= 2);
    assert!((points[0].logical - 2.0).abs() < 1e-6);
    assert!((points[0].price - 10.0).abs() < 1e-6);
    for (anchor, &(x, y)) in points.iter().zip(captured.iter()) {
        let px_x = chart.logical_to_coordinate(anchor.logical).unwrap();
        let px_y = chart.series_price_to_coordinate(0, anchor.price).unwrap();
        assert!((px_x - x).abs() < 1e-6 && (px_y - y).abs() < 1e-6);
    }
    // The stroke is left selected (reference-informed behavior).
    assert_eq!(chart.selected_drawing(), Some(id));
}

#[test]
fn rejected_brush_samples_do_not_rebuild_the_drawings_layer() {
    let mut chart = settled_chart();
    let start = (x_at(&chart, 2.0), y_at(&chart, 10.0));
    assert!(chart.brush_create_start(None, start.0, start.1));
    // Absorb the stroke-start invalidation before measuring.
    chart.build_frame();
    // Sub-threshold jitter: rejected, and the drawings layer must stay clean so hosts repainting
    // per pointer event don't rebuild the scene for nothing.
    assert!(!chart.brush_create_add(start.0 + 0.5, start.1 + 0.5));
    chart.build_frame();
    assert_eq!(chart.frame_build_stats().drawing_rebuilds, 0);
    // A real sample is captured and dirties the layer.
    assert!(chart.brush_create_add(start.0 + 30.0, start.1));
    chart.build_frame();
    assert_eq!(chart.frame_build_stats().drawing_rebuilds, 1);
}

#[test]
fn live_options_update_brush_without_losing_captured_points() {
    let mut chart = settled_chart();
    let start = (x_at(&chart, 2.0), y_at(&chart, 10.0));
    assert!(chart.brush_create_start(
        Some(r##"{"color":"#123456","width":2.0}"##),
        start.0,
        start.1,
    ));
    chart.brush_create_add(x_at(&chart, 4.0), y_at(&chart, 11.0));
    let points_before = chart.brush_capture().unwrap().points.clone();

    assert!(chart.drawing_create_apply_options(r##"{"width":5.0,"style":"dotted"}"##));
    let capture = chart.brush_capture().unwrap();
    assert_eq!(capture.points, points_before);
    assert_eq!(capture.options.color, "#123456");
    assert_eq!(capture.options.width, 5.0);
    assert_eq!(capture.options.style, LineStyle::Dotted);

    let id = chart.brush_create_end();
    let drawing = chart.drawing(id).unwrap();
    assert_eq!(drawing.color, "#123456");
    assert_eq!(drawing.width, 5.0);
    assert!(drawing.points.len() >= 2);
}

#[test]
fn brush_capture_end_discards_a_degenerate_stroke() {
    let mut chart = settled_chart();
    assert!(chart.brush_create_start(None, x_at(&chart, 2.0), y_at(&chart, 10.0)));
    // A click without a drag: one point only — no drawing is committed.
    assert_eq!(chart.brush_create_end(), 0);
    assert_eq!(chart.drawings().len(), 0);
    // No active capture: end is a no-op.
    assert_eq!(chart.brush_create_end(), 0);
    // Cancel drops the capture too.
    assert!(chart.brush_create_start(None, x_at(&chart, 2.0), y_at(&chart, 10.0)));
    chart.brush_create_cancel();
    assert!(!chart.brush_create_active());
}

// --- text tool: placeholder + container ---

/// The text prims emitted for the current drawings (text + color) and the container boxes
/// (fill colors and border frames), from a fresh frame.
type TextPrims = (Vec<(String, Color)>, Vec<(Color, Option<(i32, Color)>)>);

fn text_prims(chart: &mut ChartEngine) -> TextPrims {
    let frame = chart.build_frame();
    let mut texts = Vec::new();
    let mut fills = Vec::new();
    let mut frame_border = None;
    for prim in &frame.panes[0].main {
        match prim {
            Prim::Text { text, color, .. } => texts.push((text.clone(), *color)),
            Prim::Rect { color, .. } => fills.push(*color),
            Prim::RectFrame { border, color, .. } => frame_border = Some((*border, *color)),
            _ => {}
        }
    }
    let boxes = fills.into_iter().map(|fill| (fill, frame_border)).collect();
    (texts, boxes)
}

#[test]
fn empty_text_tool_paints_nothing_but_remains_hittable() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            None,
        )
        .unwrap();
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.is_empty(), "empty text tools paint no canvas ghost");
    // The caret-sized chrome box is still a click target (host opens the editor).
    let hit = chart
        .hit_test_drawing(x_at(&chart, 5.0), y_at(&chart, 11.0))
        .expect("empty text is clickable");
    assert_eq!(hit.id, id);
    assert!(chart.drawing_apply_options(id, r##"{"text":"hello"}"##));
    let (texts, _) = text_prims(&mut chart);
    let (_, color) = texts
        .iter()
        .find(|(t, _)| t == "hello")
        .expect("label rendered");
    assert_eq!(color.a(), 255);
}

#[test]
fn text_styling_options_round_trip_and_render() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r##"{"text":"styled","text_weight":600,"text_italic":true,"text_color":"#ff0000"}"##),
        )
        .unwrap();
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(id).unwrap()).unwrap();
    assert_eq!(options["text_weight"], 600);
    assert_eq!(options["text_italic"], true);
    // The legacy boolean derives from the weight (semibold and up reads bold).
    assert_eq!(options["text_bold"], true);
    assert_eq!(options["text_color"], "#ff0000");
    // The emitted Prim::Text carries the weight and the italic flag.
    let frame = chart.build_frame();
    let (weight, italic) = frame.panes[0]
        .main
        .iter()
        .find_map(|prim| match prim {
            Prim::Text {
                text,
                weight,
                italic,
                ..
            } if text == "styled" => Some((*weight, *italic)),
            _ => None,
        })
        .expect("label rendered");
    assert_eq!((weight, italic), (600, true));
    // The legacy `text_bold` shorthand maps to 700; an explicit weight wins and is validated.
    assert!(chart.drawing_apply_options(id, r##"{"text_bold":true}"##));
    assert_eq!(chart.drawing(id).unwrap().text_weight, Some(700));
    assert!(chart.drawing_apply_options(id, r##"{"text_weight":500}"##));
    assert_eq!(chart.drawing(id).unwrap().text_weight, Some(500));
    assert!(chart.drawing_apply_options(id, r##"{"text_weight":99}"##));
    assert_eq!(
        chart.drawing(id).unwrap().text_weight,
        Some(500),
        "out of range ignored"
    );
    assert!(chart.drawing_apply_options(id, r##"{"text_bold":false}"##));
    assert_eq!(
        chart.drawing(id).unwrap().text_weight,
        None,
        "bold off resets to normal"
    );
}

#[test]
fn editing_drawing_keeps_the_label_for_overlay_caret() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r##"{"text":"live"}"##),
        )
        .unwrap();
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "live"));
    // Typing mode: the host wrap owns the border, but the canvas label stays so the
    // transparent editor cannot lift/recolor the glyphs.
    assert!(chart.begin_drawing_text_edit(id, false));
    assert_eq!(chart.editing_drawing(), Some(id));
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "live"));
    assert!(chart.commit_drawing_text_edit());
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "live"));
}

#[test]
fn typing_into_a_text_drawing_is_one_undo_step_and_locked_text_never_edits() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r##"{"text":"a"}"##),
        )
        .unwrap();
    assert!(chart.begin_drawing_text_edit(id, false));
    for text in ["ab", "abc", "abcd"] {
        assert!(chart.set_drawing_text_edit(text, usize::MAX));
    }
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "abcd"), "live text paints");
    assert!(chart.commit_drawing_text_edit());
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().text, "a", "one step per edit");
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none(), "the next step is the creation");
    assert!(chart.redo_drawing() && chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().text, "abcd");

    assert!(chart.drawing_apply_options(id, r#"{"locked":true}"#));
    assert!(!chart.drawing_text_editable(id));
    assert!(!chart.begin_drawing_text_edit(id, false));
    assert_eq!(chart.editing_drawing(), None);
}

/// Whether any part of the layout's text box is inside the pane's plot, the rule the editor
/// refuses against: a drawing is clipped to its pane, so text wholly outside paints nothing.
fn text_in_its_pane(chart: &ChartEngine, id: DrawingId) -> bool {
    let layout = chart.drawing_text_edit_layout(id).unwrap();
    let pane = &chart.panes[chart.drawing(id).unwrap().pane_index];
    let [left, top, right, bottom] = layout.rect;
    right > 0.0 && left < chart.pane_w && bottom > pane.top && top < pane.top + pane.height
}

#[test]
fn text_wholly_outside_the_plot_is_not_editable_and_never_opens() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r##"{"text":"live"}"##),
        )
        .unwrap();
    // Pan the text across the left edge a quarter bar at a time: it stays editable exactly
    // while any part of its box is inside the plot (including straddling the edge), and a
    // refused begin leaves no session behind.
    let (mut inside, mut outside) = (0, 0);
    for step in 0..400 {
        chart.set_right_offset(f64::from(step) * 0.25);
        chart.build_frame();
        let expected = text_in_its_pane(&chart, id);
        assert_eq!(chart.drawing_text_editable(id), expected, "step {step}");
        assert_eq!(chart.begin_drawing_text_edit(id, false), expected);
        assert_eq!(chart.editing_drawing(), expected.then_some(id));
        chart.cancel_drawing_text_edit();
        if expected {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    assert!(
        inside > 0 && outside > 0,
        "{inside} inside, {outside} outside"
    );

    // Vertically too: a price far above the scale puts the text above the plot.
    chart.set_right_offset(0.0);
    let high = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 1.0e6,
            }],
            Some(r##"{"text":"high"}"##),
        )
        .unwrap();
    chart.build_frame();
    assert!(!text_in_its_pane(&chart, high));
    assert!(!chart.drawing_text_editable(high));
    assert!(!chart.begin_drawing_text_edit(high, false));
    assert_eq!(chart.editing_drawing(), None);
}

#[test]
fn text_over_another_pane_is_outside_its_own_pane_and_not_editable() {
    let mut chart = settled_chart();
    let lower = chart.add_pane(true).unwrap();
    let series = chart.add_series(SeriesKind::Line);
    // The same bar times as the candles, so the logical slots keep their meaning.
    let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
    let values = vec![10.0; 10];
    chart
        .set_series_data(series, &times, &values, &values, &values, &values)
        .unwrap();
    chart.set_series_pane(series, lower, 1.0);
    chart.build_frame();
    // A price on the lower pane's scale that lands in the middle of the upper pane: the text box
    // is inside the chart, but the lower pane clips its drawings, so nothing of it paints.
    let (x, y) = (
        chart.time_scale.logical_to_coordinate(5.0),
        chart.panes[0].top + chart.panes[0].height / 2.0,
    );
    let price = chart
        .drawing_from_px_for(lower, crate::DrawingPriceScale::Right, x, y)
        .unwrap()
        .price;
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            lower,
            vec![DrawingPoint {
                logical: 5.0,
                price,
            }],
            Some(r##"{"text":"stray"}"##),
        )
        .unwrap();
    chart.build_frame();
    let layout = chart.drawing_text_edit_layout(id).unwrap();
    let upper = &chart.panes[0];
    assert!(layout.rect[1] >= upper.top && layout.rect[3] <= upper.top + upper.height);
    assert!(
        layout.rect[0] > 0.0 && layout.rect[2] < chart.pane_w,
        "{:?}",
        layout.rect
    );
    assert!(!chart.drawing_text_editable(id));
    assert!(!chart.begin_drawing_text_edit(id, false));
    // On its own pane the same text edits.
    let own = chart
        .add_drawing(
            DrawingKind::Text,
            lower,
            vec![DrawingPoint {
                logical: 5.0,
                price: 10.0,
            }],
            Some(r##"{"text":"own"}"##),
        )
        .unwrap();
    chart.build_frame();
    assert!(chart.drawing_text_editable(own));
    assert!(chart.begin_drawing_text_edit(own, false));
}

#[test]
fn text_tool_container_draws_a_crisp_box_behind_the_run() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![DrawingPoint {
                logical: 5.0,
                price: 11.0,
            }],
            Some(r##"{"text":"boxed","box_color":"rgba(255, 0, 0, 0.5)","box_border_color":"#0000ff","box_border_width":2}"##),
        )
        .unwrap();
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "boxed"));
    // Exactly one container fill (candle rects carry their own colors). Idle drawings paint
    // below price series (ordering.rs), so the container border is no longer the frame's last
    // RectFrame — search for it explicitly rather than relying on trailing order.
    let frame = chart.build_frame();
    let fills = frame.panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::Rect { color, .. } if *color == Color::rgba(0xff, 0, 0, 0x80)))
        .count();
    assert_eq!(fills, 1);
    assert!(frame.panes[0].main.iter().any(|prim| matches!(
        prim,
        Prim::RectFrame { border, color, .. } if *border == 2 && *color == Color::rgb(0, 0, 0xff)
    )));
    // Options round-trip the container settings.
    let options: serde_json::Value =
        serde_json::from_str(&chart.drawing_options_json(id).unwrap()).unwrap();
    assert_eq!(options["box_color"], "rgba(255, 0, 0, 0.5)");
    assert_eq!(options["box_border_color"], "#0000ff");
    assert_eq!(options["box_border_width"], 2.0);
    // Clearing both removes the box (candle rects are unaffected, of course).
    assert!(chart.drawing_apply_options(id, r##"{"box_color":"","box_border_color":""}"##));
    let (_, boxes) = text_prims(&mut chart);
    assert!(boxes
        .iter()
        .all(|(fill, _)| *fill != Color::rgba(0xff, 0, 0, 0x80)));
}

#[test]
fn thousand_mostly_offscreen_drawings_bound_frame_and_hit_work() {
    let mut chart = settled_chart();
    for index in 0..1_000 {
        let logical = if index < 10 {
            2.0 + index as f64 * 0.4
        } else {
            1_000.0 + index as f64 * 10.0
        };
        chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical,
                        price: 10.5,
                    },
                    DrawingPoint {
                        logical: logical + 0.25,
                        price: 11.0,
                    },
                ],
                None,
            )
            .unwrap();
    }

    chart.reset_drawing_work_stats();
    chart.build_frame();
    let frame = chart.drawing_work_stats();
    assert_eq!(frame.drawings_total, 1_000);
    assert_eq!(frame.candidates, 10);
    assert_eq!(frame.visible, 10);
    assert_eq!(frame.geometry_rebuilds, 10);

    chart.reset_drawing_work_stats();
    assert_eq!(chart.hit_test_drawing(5.0, 5.0), None);
    let hit = chart.drawing_work_stats();
    assert_eq!(hit.drawings_total, 1_000);
    assert_eq!(hit.candidates, 0);
    assert_eq!(hit.precise_hit_tests, 0);
}

#[test]
fn every_tool_has_conservative_viewport_inclusion_and_clear_exclusion() {
    let mut chart = settled_chart();
    for kind in [
        DrawingKind::TrendLine,
        DrawingKind::HorizontalLine,
        DrawingKind::HorizontalRay,
        DrawingKind::VerticalLine,
        DrawingKind::Rectangle,
        DrawingKind::Text,
        DrawingKind::Brush,
        DrawingKind::Path,
        DrawingKind::LongPosition,
        DrawingKind::ShortPosition,
    ] {
        let visible = match kind {
            DrawingKind::TrendLine
            | DrawingKind::Rectangle
            | DrawingKind::Brush
            | DrawingKind::Path => vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 10.5,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 12.5,
                },
            ],
            DrawingKind::LongPosition | DrawingKind::ShortPosition => vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 11.0,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 13.0,
                },
                DrawingPoint {
                    logical: 7.0,
                    price: 10.0,
                },
            ],
            _ => vec![DrawingPoint {
                logical: 4.0,
                price: 11.0,
            }],
        };
        let visible_id = chart.add_drawing(kind, 0, visible, None).unwrap();
        assert!(chart.drawing_viewport_candidate_reference(chart.drawing(visible_id).unwrap()));

        let offscreen = match kind {
            DrawingKind::HorizontalLine => vec![DrawingPoint {
                logical: 0.0,
                price: 1_000.0,
            }],
            DrawingKind::TrendLine
            | DrawingKind::Rectangle
            | DrawingKind::Brush
            | DrawingKind::Path => vec![
                DrawingPoint {
                    logical: 1_000.0,
                    price: 1_000.0,
                },
                DrawingPoint {
                    logical: 1_010.0,
                    price: 1_010.0,
                },
            ],
            DrawingKind::LongPosition | DrawingKind::ShortPosition => vec![
                DrawingPoint {
                    logical: 1_000.0,
                    price: 1_000.0,
                },
                DrawingPoint {
                    logical: 1_010.0,
                    price: 1_020.0,
                },
                DrawingPoint {
                    logical: 1_010.0,
                    price: 990.0,
                },
            ],
            _ => vec![DrawingPoint {
                logical: 1_000.0,
                price: 1_000.0,
            }],
        };
        let offscreen_id = chart.add_drawing(kind, 0, offscreen, None).unwrap();
        assert!(
            !chart.drawing_viewport_candidate_reference(chart.drawing(offscreen_id).unwrap()),
            "{kind:?}"
        );
    }
}

#[test]
fn thousand_overlapping_drawings_preserve_the_honest_linear_worst_case() {
    let mut chart = settled_chart();
    for _ in 0..1_000 {
        chart
            .add_drawing(
                DrawingKind::Rectangle,
                0,
                vec![
                    DrawingPoint {
                        logical: 2.0,
                        price: 10.0,
                    },
                    DrawingPoint {
                        logical: 7.0,
                        price: 13.0,
                    },
                ],
                None,
            )
            .unwrap();
    }
    let x = (x_at(&chart, 2.0) + x_at(&chart, 7.0)) / 2.0;
    let y = (y_at(&chart, 10.0) + y_at(&chart, 13.0)) / 2.0;
    chart.reset_drawing_work_stats();
    assert_eq!(chart.hit_test_drawing(x, y), None);
    let work = chart.drawing_work_stats();
    assert_eq!(work.candidates, 1_000);
    assert_eq!(work.precise_hit_tests, 1_000);
}

#[test]
fn long_offscreen_brush_uses_cached_bounds_without_rebuilding_path_geometry() {
    let mut chart = settled_chart();
    let points = (0..10_000)
        .map(|index| DrawingPoint {
            logical: 1_000.0 + index as f64 * 0.01,
            price: 10.0 + (index as f64 * 0.01).sin(),
        })
        .collect();
    chart
        .add_drawing(DrawingKind::Brush, 0, points, None)
        .unwrap();
    // Add enough ordinary drawings to engage the indexed frame path.
    for index in 0..21 {
        chart
            .add_drawing(
                DrawingKind::VerticalLine,
                0,
                vec![DrawingPoint {
                    logical: 2_000.0 + index as f64,
                    price: 10.0,
                }],
                None,
            )
            .unwrap();
    }
    chart.reset_drawing_work_stats();
    chart.build_frame();
    let frame = chart.drawing_work_stats();
    assert_eq!(frame.candidates, 0);
    assert_eq!(frame.geometry_rebuilds, 0);
    chart.reset_drawing_work_stats();
    assert_eq!(chart.hit_test_drawing(400.0, 250.0), None);
    assert_eq!(chart.drawing_work_stats().geometry_rebuilds, 0);
}

#[test]
fn indexed_hit_matches_bruteforce_for_randomized_catalog() {
    let mut chart = settled_chart();
    let kinds = [
        DrawingKind::TrendLine,
        DrawingKind::HorizontalLine,
        DrawingKind::HorizontalRay,
        DrawingKind::VerticalLine,
        DrawingKind::Rectangle,
        DrawingKind::Text,
        DrawingKind::Brush,
        DrawingKind::Path,
        DrawingKind::LongPosition,
        DrawingKind::ShortPosition,
    ];
    let mut state = 0x9e37_79b9_u32;
    let mut random = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        f64::from(state) / f64::from(u32::MAX)
    };
    for index in 0..400 {
        let kind = kinds[index % kinds.len()];
        let first = DrawingPoint {
            logical: random() * 30.0 - 10.0,
            price: random() * 12.0 + 5.0,
        };
        let mut points = vec![first];
        if matches!(
            kind,
            DrawingKind::TrendLine
                | DrawingKind::Rectangle
                | DrawingKind::Brush
                | DrawingKind::Path
                | DrawingKind::LongPosition
                | DrawingKind::ShortPosition
        ) {
            points.push(DrawingPoint {
                logical: first.logical + random() * 8.0,
                price: first.price + random() * 5.0 - 2.5,
            });
        }
        if matches!(kind, DrawingKind::LongPosition | DrawingKind::ShortPosition) {
            points.push(DrawingPoint {
                logical: first.logical + random() * 8.0,
                price: first.price + random() * 5.0 - 2.5,
            });
        }
        chart
            .add_drawing(
                kind,
                0,
                points,
                (kind == DrawingKind::Text).then_some(r#"{"text":"random label"}"#),
            )
            .unwrap();
    }
    for index in 0..2_000 {
        let x = random() * chart.pane_w;
        let y = random() * chart.pane_h;
        assert_eq!(
            chart.hit_test_drawing(x, y),
            chart.hit_test_drawing_bruteforce(x, y),
            "pointer {index} at ({x}, {y})"
        );
    }
}

#[test]
fn candidate_frame_matches_full_reference_across_viewports_and_mutations() {
    let mut chart = settled_chart();
    let kinds = [
        DrawingKind::TrendLine,
        DrawingKind::HorizontalLine,
        DrawingKind::HorizontalRay,
        DrawingKind::VerticalLine,
        DrawingKind::Rectangle,
        DrawingKind::Text,
        DrawingKind::Brush,
        DrawingKind::Path,
        DrawingKind::LongPosition,
        DrawingKind::ShortPosition,
    ];
    let mut ids = Vec::new();
    for index in 0..210 {
        let kind = kinds[index % kinds.len()];
        let logical = index as f64 - 100.0;
        let mut points = vec![DrawingPoint {
            logical,
            price: 8.0 + (index % 50) as f64 * 0.1,
        }];
        if matches!(
            kind,
            DrawingKind::TrendLine
                | DrawingKind::Rectangle
                | DrawingKind::Brush
                | DrawingKind::Path
                | DrawingKind::LongPosition
                | DrawingKind::ShortPosition
        ) {
            points.push(DrawingPoint {
                logical: logical + 4.0,
                price: points[0].price + 1.0,
            });
        }
        if matches!(kind, DrawingKind::LongPosition | DrawingKind::ShortPosition) {
            points.push(DrawingPoint {
                logical: logical + 4.0,
                price: points[0].price - 1.0,
            });
        }
        ids.push(
            chart
                .add_drawing(
                    kind,
                    0,
                    points,
                    (kind == DrawingKind::Text).then_some(r#"{"text":"parity"}"#),
                )
                .unwrap(),
        );
    }

    let assert_parity = |chart: &ChartEngine| {
        let mut optimized_prims = Vec::new();
        let mut optimized_points = Vec::new();
        let mut discard_parts = Vec::new();
        let mut discard_preview = (0usize, 0usize);
        chart.build_drawings_frame_segmented(
            0,
            chart.pane_w.round() as i32,
            1.0,
            1.0,
            &mut optimized_prims,
            &mut optimized_points,
            &mut discard_parts,
            &mut discard_preview,
        );
        let mut reference_prims = Vec::new();
        let mut reference_points = Vec::new();
        chart.build_drawings_frame_reference(
            0,
            chart.pane_w.round() as i32,
            1.0,
            1.0,
            &mut reference_prims,
            &mut reference_points,
        );
        assert_eq!(optimized_prims, reference_prims);
        assert_eq!(optimized_points, reference_points);
    };

    for (from, to) in [(-120.0, -80.0), (-10.0, 10.0), (80.0, 120.0)] {
        chart.set_visible_logical_range(from, to);
        chart.build_frame();
        assert_parity(&chart);
    }
    assert!(chart.drawing_apply_options(ids[70], r#"{"width":8,"text":"changed"}"#));
    assert!(chart.drawing_set_points(
        ids[140],
        r#"[{"logical":0,"price":10},{"logical":8,"price":12}]"#,
    ));
    assert!(chart.remove_drawing(ids[35]));
    chart.dpr = 2.0;
    chart.css_width = 960.0;
    chart.pane_w = 960.0;
    chart.build_frame();
    assert_parity(&chart);
}

#[test]
fn pane_candidates_are_isolated_and_removed_panes_drop_membership() {
    let mut chart = settled_chart();
    for _ in 1..4 {
        let pane = chart.add_pane(true).unwrap();
        let series = chart.add_series(SeriesKind::Line);
        let times = (0..10).map(|index| index as f64).collect::<Vec<_>>();
        let values = vec![10.0; 10];
        chart
            .set_series_data(series, &times, &values, &values, &values, &values)
            .unwrap();
        chart.set_series_pane(series, pane, 1.0);
    }
    for pane in 0..4 {
        for index in 0..250 {
            chart
                .add_drawing(
                    DrawingKind::VerticalLine,
                    pane,
                    vec![DrawingPoint {
                        logical: index as f64,
                        price: 10.0,
                    }],
                    None,
                )
                .unwrap();
        }
    }
    chart.build_frame();
    let y = chart.panes[2].top + chart.panes[2].height / 2.0;
    chart.reset_drawing_work_stats();
    let _ = chart.hit_test_drawing(7.0, y);
    assert_eq!(chart.drawing_work_stats().drawings_total, 250);

    assert!(chart.remove_pane(2));
    assert!(
        chart
            .drawings()
            .iter()
            .filter(|drawing| drawing.pane_index == PANELESS)
            .count()
            == 250
    );
    chart.reset_drawing_work_stats();
    let _ = chart.hit_test_drawing(7.0, chart.panes[0].height / 2.0);
    assert_eq!(chart.drawing_work_stats().drawings_total, 250);
}

#[test]
fn selection_crosshair_and_one_drawing_drag_keep_unrelated_geometry_retained() {
    let mut chart = settled_chart();
    let mut first = 0;
    for index in 0..100 {
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 2.0 + index as f64 * 0.01,
                        price: 10.5,
                    },
                    DrawingPoint {
                        logical: 7.0,
                        price: 12.5,
                    },
                ],
                None,
            )
            .unwrap();
        if index == 0 {
            first = id;
        }
    }
    chart.build_frame();

    chart.set_selected_drawing(Some(first));
    chart.build_frame();
    let frame = chart.frame_build_stats();
    assert_eq!(frame.drawing_rebuilds, 0);
    assert_eq!(frame.overlay_rebuilds, 1);

    chart.reset_drawing_work_stats();
    chart.set_crosshair_at(400.0, 250.0);
    chart.build_frame();
    assert_eq!(chart.frame_build_stats().drawing_rebuilds, 0);
    assert_eq!(chart.drawing_work_stats().geometry_rebuilds, 0);

    let (x, y) = chart.drawing_point_to_coordinate(first, 0).unwrap();
    assert!(chart.drawing_drag_start_at(x, y));
    chart.reset_drawing_work_stats();
    chart.drawing_drag_to(x + 2.0, y + 1.0, DrawingModifiers::default());
    chart.build_frame();
    let work = chart.drawing_work_stats();
    assert_eq!(work.bounds_rebuilds, 1);
    assert_eq!(work.geometry_rebuilds, 1);
}

#[test]
fn drawing_ids_never_wrap_into_a_live_or_sentinel_identity() {
    let mut chart = settled_chart();
    chart.next_drawing_id = DrawingId::MAX;
    assert!(chart
        .add_drawing(
            DrawingKind::VerticalLine,
            0,
            vec![DrawingPoint {
                logical: 1.0,
                price: 10.0,
            }],
            None,
        )
        .is_none());
    assert!(chart.drawings().is_empty());
    assert_eq!(chart.next_drawing_id, DrawingId::MAX);
}

#[test]
fn drawing_runtime_is_isolated_per_chart_even_when_ids_overlap() {
    let mut first = settled_chart();
    let mut second = settled_chart();
    for index in 0..30 {
        first
            .add_drawing(
                DrawingKind::VerticalLine,
                0,
                vec![DrawingPoint {
                    logical: index as f64,
                    price: 10.0,
                }],
                None,
            )
            .unwrap();
    }
    for index in 0..45 {
        second
            .add_drawing(
                DrawingKind::VerticalLine,
                0,
                vec![DrawingPoint {
                    logical: index as f64,
                    price: 10.0,
                }],
                None,
            )
            .unwrap();
    }

    first.reset_drawing_work_stats();
    second.reset_drawing_work_stats();
    first.build_frame();
    assert_eq!(first.drawing_work_stats().drawings_total, 30);
    assert_eq!(second.drawing_work_stats(), DrawingWorkStats::default());

    second.build_frame();
    assert_eq!(second.drawing_work_stats().drawings_total, 45);
    first.clear_drawings();
    assert_eq!(second.drawings().len(), 45);
}

#[test]
fn data_reading_drawings_measure_the_first_ordinary_series_on_their_scale() {
    let mut chart = ohlc_chart();
    let sma = chart.add_sma(0, 2).expect("SMA output");
    let line = chart.add_series(SeriesKind::Line);
    let custom = chart.add_series(SeriesKind::Custom);
    let forecast = Drawing::new(1, DrawingKind::Forecast, 0, Vec::new());
    assert_eq!(chart.drawing_source_series(&forecast), Some(0));
    // Neither paint order nor visibility moves the source.
    assert!(chart.set_series_order(vec![line, custom, sma, 0]));
    chart.set_series_visible(0, false);
    assert_eq!(chart.drawing_source_series(&forecast), Some(0));
    // Without it, the next ordinary series takes over; indicator outputs and custom series
    // never do, and another scale or pane has its own source.
    assert!(chart.remove_series(0));
    assert_eq!(chart.drawing_source_series(&forecast), Some(line));
    assert!(chart.remove_series(line));
    assert_eq!(chart.drawing_source_series(&forecast), None);
    let mut left = forecast.clone();
    left.price_scale = DrawingPriceScale::Left;
    assert_eq!(chart.drawing_source_series(&left), None);
}

#[test]
fn data_reading_drawings_keep_creation_order_when_storage_slots_are_reused() {
    let forecast = Drawing::new(1, DrawingKind::Forecast, 0, Vec::new());
    // One removal: a later series reuses the removed primary's slot but stays newer.
    let mut chart = ohlc_chart();
    let older = chart.add_series(SeriesKind::Line);
    assert!(chart.remove_series(0));
    let newer = chart.add_series(SeriesKind::Line);
    assert!(newer > older);
    assert_eq!(chart.drawing_source_series(&forecast), Some(older));

    // A remount of two series: freed slots come back last-in-first-out, so the series added
    // first lands in the higher slot and must still be the source.
    let mut chart = ohlc_chart();
    let candles = chart.add_series(SeriesKind::Candlestick);
    let line = chart.add_series(SeriesKind::Line);
    assert!(chart.remove_series(0));
    assert!(chart.remove_series(candles));
    assert!(chart.remove_series(line));
    let remounted_candles = chart.add_series(SeriesKind::Candlestick);
    let remounted_line = chart.add_series(SeriesKind::Line);
    assert!(remounted_line > remounted_candles);
    assert_eq!(
        chart.drawing_source_series(&forecast),
        Some(remounted_candles)
    );
}

// --- inline text editing coverage (`drawing_text_hit_at`, run layout, tool-owned editing) -----

fn pt(logical: f64, price: f64) -> DrawingPoint {
    DrawingPoint { logical, price }
}

/// The media-px center of the box between two anchors.
fn box_center(chart: &ChartEngine, a: DrawingPoint, b: DrawingPoint) -> (f64, f64) {
    (
        (x_at(chart, a.logical) + x_at(chart, b.logical)) / 2.0,
        (y_at(chart, a.price) + y_at(chart, b.price)) / 2.0,
    )
}

#[test]
fn text_hit_reaches_the_label_of_a_shape_whose_interior_is_not_a_body_hit() {
    let mut chart = settled_chart();
    let (a, b) = (pt(2.0, 10.0), pt(7.0, 13.0));
    let (cx, cy) = box_center(&chart, a, b);
    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![a, b],
            Some(r#"{"text":"box"}"#),
        )
        .unwrap();
    assert_eq!(
        chart.hit_test_drawing(cx, cy),
        None,
        "an unselected rectangle's interior stays a pan surface"
    );
    assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(rectangle));
    // The rest of the interior is not the label.
    assert_eq!(chart.drawing_text_hit_at(cx, cy + 60.0), None);
    assert_eq!(chart.drawing_text_hit_at(cx + 200.0, cy), None);

    let ellipse = chart
        .add_drawing(
            DrawingKind::Ellipse,
            0,
            vec![pt(1.0, 10.0), pt(8.0, 13.0)],
            Some(r#"{"text":"oval","text_size":20}"#),
        )
        .unwrap();
    let (ex, ey) = box_center(&chart, pt(1.0, 10.0), pt(8.0, 13.0));
    assert_eq!(
        chart.drawing_text_hit_at(ex, ey),
        Some(ellipse),
        "the topmost label wins"
    );
}

#[test]
fn text_hit_follows_a_rotated_segment_label_and_requires_text_off_the_trend_line() {
    let mut chart = settled_chart();
    let ray = chart
        .add_drawing(
            DrawingKind::Ray,
            0,
            vec![pt(1.0, 10.0), pt(6.0, 12.5)],
            Some(r#"{"text":"rotated ray label","text_h_align":"center","text_v_align":"middle"}"#),
        )
        .unwrap();
    let (x, y, angle) = chart.drawing_text_transform(ray).unwrap();
    assert!(angle.abs() > 0.25, "the fixture must rotate the run");
    let along = 30.0;
    assert_eq!(
        chart.drawing_text_hit_at(x + angle.cos() * along, y + angle.sin() * along),
        Some(ray)
    );
    assert_eq!(
        chart.drawing_text_hit_at(x + along, y),
        None,
        "the screen-horizontal spot is outside the rotated run"
    );

    // Only a trend line prompts `+ Add text`; an empty label elsewhere has no hit region.
    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![pt(2.0, 10.0), pt(7.0, 13.0)],
            None,
        )
        .unwrap();
    let (cx, cy) = box_center(&chart, pt(2.0, 10.0), pt(7.0, 13.0));
    assert_eq!(chart.drawing_text_hit_at(cx, cy), None);
    assert!(chart.drawing_apply_options(rectangle, r#"{"text":"x"}"#));
    assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(rectangle));
    assert!(chart.drawing_apply_options(rectangle, r#"{"text":""}"#));
    let trend = add_trend(&mut chart);
    let (tx, ty) = chart.drawing_text_coordinate(trend).unwrap();
    assert_eq!(
        chart.drawing_text_hit_at(tx - 20.0, ty),
        Some(trend),
        "an empty trend label keeps its placeholder region"
    );
}

#[test]
fn text_hit_skips_locked_hidden_and_interval_hidden_drawings_and_bounds_the_pointer() {
    let mut chart = settled_chart();
    let (a, b) = (pt(2.0, 10.0), pt(7.0, 13.0));
    let (cx, cy) = box_center(&chart, a, b);
    let id = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![a, b],
            Some(r#"{"text":"box"}"#),
        )
        .unwrap();
    assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(id));
    for patch in [
        r#"{"locked":true}"#,
        r#"{"visible":false}"#,
        r#"{"interval_visibility":{"enabled":true,"intervals":[]}}"#,
    ] {
        assert!(chart.drawing_apply_options(id, patch), "{patch}");
        assert_eq!(chart.drawing_text_hit_at(cx, cy), None, "{patch}");
        assert!(chart.undo_drawing());
        assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(id), "{patch}");
    }
    for (x, y) in [(f64::NAN, cy), (cx, f64::INFINITY)] {
        assert_eq!(chart.drawing_text_hit_at(x, y), None);
    }

    // A label overhanging the pane edge is not hittable from the price-axis side.
    let edge = chart.coordinate_to_logical(chart.pane_w - 5.0).unwrap();
    let vline = chart
        .add_drawing(
            DrawingKind::VerticalLine,
            0,
            vec![pt(edge, 11.0)],
            Some(r#"{"text":"overhanging label"}"#),
        )
        .unwrap();
    let y = chart.drawing_text_coordinate(vline).unwrap().1;
    assert_eq!(
        chart.drawing_text_hit_at(chart.pane_w - 10.0, y),
        Some(vline)
    );
    assert_eq!(chart.drawing_text_hit_at(chart.pane_w + 10.0, y), None);
    assert_eq!(chart.drawing_text_hit_at(-10.0, y), None);
}

#[test]
fn text_tool_and_annotation_boxes_stay_body_hits_not_label_hits() {
    let mut chart = settled_chart();
    let text = chart
        .add_drawing(
            DrawingKind::Text,
            0,
            vec![pt(4.0, 11.0)],
            Some(r#"{"text":"levels"}"#),
        )
        .unwrap();
    let (x, y) = chart.drawing_text_coordinate(text).unwrap();
    assert_eq!(chart.hit_test_drawing(x, y).map(|hit| hit.id), Some(text));
    assert_eq!(chart.drawing_text_hit_at(x, y), None);
    let comment = chart
        .add_drawing(DrawingKind::Comment, 0, vec![pt(6.0, 12.0)], None)
        .unwrap();
    let layout = chart.drawing_text_edit_layout(comment).unwrap();
    let [left, top, right, bottom] = layout.rect;
    let center = ((left + right) / 2.0, (top + bottom) / 2.0);
    assert_eq!(
        chart.hit_test_drawing(center.0, center.1).map(|hit| hit.id),
        Some(comment)
    );
    assert_eq!(chart.drawing_text_hit_at(center.0, center.1), None);
}

#[test]
fn a_higher_body_or_the_selected_handle_suppresses_a_lower_label_hit() {
    let mut chart = settled_chart();
    // The box's center sits on a bar (logical 5) and mid price, so an anchor can land on it.
    let (a, b) = (pt(2.0, 10.0), pt(8.0, 13.0));
    let (cx, cy) = box_center(&chart, a, b);
    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![a, b],
            Some(r#"{"text":"box"}"#),
        )
        .unwrap();
    assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(rectangle));

    // A higher-z trend line passing through the label covers it: the body wins.
    let cover = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![pt(2.0, 11.5), pt(8.0, 11.5)],
            None,
        )
        .unwrap();
    assert_eq!(
        chart.hit_test_drawing(cx, cy).map(|hit| hit.id),
        Some(cover)
    );
    assert_eq!(chart.drawing_text_hit_at(cx, cy), None);

    // A label on top of the covering body wins.
    assert!(chart.move_drawing_z_order(rectangle, 1));
    assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(rectangle));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing_text_hit_at(cx, cy), None);
    assert!(chart.remove_drawing(cover));
    assert_eq!(chart.drawing_text_hit_at(cx, cy), Some(rectangle));

    // The selected drawing's anchor handle at the label wins over the label beneath it. The
    // probe sits just left of the anchor: inside the handle's radius but outside the stroke.
    let handle_owner = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![pt(5.0, 11.5), pt(9.0, 12.0)],
            None,
        )
        .unwrap();
    let probe = (cx - 5.0, cy);
    assert_eq!(chart.hit_test_drawing(probe.0, probe.1), None);
    assert_eq!(chart.drawing_text_hit_at(probe.0, probe.1), Some(rectangle));
    chart.set_selected_drawing(Some(handle_owner));
    assert_eq!(
        chart
            .hit_test_drawing(probe.0, probe.1)
            .map(|hit| (hit.id, hit.part)),
        Some((handle_owner, DrawingDragPart::Anchor(0)))
    );
    assert_eq!(chart.drawing_text_hit_at(probe.0, probe.1), None);
    chart.set_selected_drawing(None);
    assert_eq!(
        chart.drawing_text_hit_at(cx, cy),
        None,
        "the trend line's stroke starts at the label center and covers it"
    );
}

#[test]
fn drawing_at_answers_what_a_click_would_select_without_moving_the_selection() {
    let mut chart = settled_chart();
    let (a, b) = (pt(2.0, 10.0), pt(8.0, 13.0));
    let (cx, cy) = box_center(&chart, a, b);
    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![a, b],
            Some(r#"{"text":"box"}"#),
        )
        .unwrap();
    let far = (chart.pane_w - 8.0, 8.0);
    let inside_below_label = (cx, cy + 60.0);

    // Unselected: only the label reaches the shape (its interior is a pan surface).
    assert_eq!(chart.selected_drawing(), None);
    assert_eq!(chart.drawing_at(cx, cy), Some(rectangle));
    assert_eq!(
        chart.drawing_at(inside_below_label.0, inside_below_label.1),
        None
    );
    assert_eq!(chart.drawing_at(far.0, far.1), None);
    assert_eq!(chart.selected_drawing(), None, "asking never selects");

    // Selected: the whole interior and the anchor handles are the drawing; elsewhere is not.
    chart.set_selected_drawing(Some(rectangle));
    assert_eq!(
        chart.drawing_at(inside_below_label.0, inside_below_label.1),
        Some(rectangle)
    );
    let corner = (x_at(&chart, a.logical), y_at(&chart, a.price));
    assert_eq!(chart.drawing_at(corner.0, corner.1), Some(rectangle));
    assert_eq!(chart.drawing_at(far.0, far.1), None);
    assert_eq!(chart.drawing_at(f64::NAN, cy), None);
    assert_eq!(chart.selected_drawing(), Some(rectangle));

    // A higher drawing's body at the point is the drawing there, not the selected one beneath.
    let cover = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![pt(2.0, 11.5), pt(8.0, 11.5)],
            None,
        )
        .unwrap();
    chart.set_selected_drawing(Some(rectangle));
    assert_eq!(chart.drawing_at(cx, cy), Some(cover));
    assert_eq!(chart.selected_drawing(), Some(rectangle));
}

#[test]
fn a_steep_short_segment_with_long_text_stays_a_candidate_along_its_whole_label() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![pt(4.0, 11.0), pt(4.02, 11.6)],
            Some(r#"{"text":"a very long steep label that runs far past the segment end"}"#),
        )
        .unwrap();
    let (x, y, angle) = chart.drawing_text_transform(id).unwrap();
    assert!(angle.abs() > 1.4, "the fixture must be steep: {angle}");
    // Walk the run: every point along its baseline is a text hit, however far from the anchors.
    let width = 60.0 * 12.0 * 0.6;
    let mut hits = 0;
    for step in 1..=10 {
        let along = -width * f64::from(step) / 11.0;
        let (px, py) = (x + angle.cos() * along, y + angle.sin() * along);
        if py < 0.0 || py > 500.0 {
            continue;
        }
        assert_eq!(
            chart.drawing_text_hit_at(px, py),
            Some(id),
            "step {step} at ({px}, {py})"
        );
        hits += 1;
    }
    assert!(hits >= 3, "the fixture must keep the run in the pane");
}

#[test]
fn drawings_without_convertible_anchors_are_not_text_editable() {
    // No data or scale: the anchors cannot convert, so no caret can be placed.
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let text = chart
        .add_drawing(DrawingKind::Text, 0, vec![pt(3.0, 11.0)], None)
        .unwrap();
    let trend = chart
        .add_drawing(
            DrawingKind::TrendLine,
            0,
            vec![pt(2.0, 10.5), pt(7.0, 12.5)],
            None,
        )
        .unwrap();
    for id in [text, trend] {
        assert!(!chart.drawing_text_editable(id));
        assert!(!chart.begin_drawing_text_edit(id, false));
    }
}

#[test]
fn placing_a_text_owning_annotation_requests_the_editor_but_price_labels_and_arrows_do_not() {
    let mut chart = settled_chart();
    let click = (x_at(&chart, 4.0), y_at(&chart, 11.0));
    let second = (x_at(&chart, 7.0), y_at(&chart, 12.5));
    for (kind, clicks, requests) in [
        (DrawingKind::Text, 1, true),
        (DrawingKind::AnchoredText, 1, true),
        (DrawingKind::Note, 1, true),
        (DrawingKind::Comment, 1, true),
        (DrawingKind::Signpost, 1, true),
        (DrawingKind::Callout, 2, true),
        (DrawingKind::PriceNote, 2, false),
        (DrawingKind::PriceLabel, 1, false),
        (DrawingKind::ArrowMarkUp, 1, false),
        (DrawingKind::ArrowMarkDown, 1, false),
        (DrawingKind::ArrowMarkLeft, 1, false),
        (DrawingKind::ArrowMarkRight, 1, false),
        (DrawingKind::FlagMark, 1, false),
        (DrawingKind::Icon, 1, false),
        (DrawingKind::Rectangle, 2, false),
        (DrawingKind::TrendLine, 2, false),
    ] {
        assert!(chart.set_drawing_tool(Some(kind), None, None), "{kind:?}");
        let mut update = if kind == DrawingKind::Text {
            chart.drawing_tool_pointer_down(click.0, click.1, DrawingModifiers::default())
        } else {
            chart.drawing_tool_activate(click.0, click.1, DrawingModifiers::default())
        };
        if clicks == 2 {
            chart.drawing_tool_pointer_move(second.0, second.1, DrawingModifiers::default(), false);
            update = chart.drawing_tool_activate(second.0, second.1, DrawingModifiers::default());
        }
        let id = update.created.unwrap_or_else(|| panic!("{kind:?} commits"));
        assert_eq!(update.request_text_edit, requests, "{kind:?}");
        assert_eq!(chart.drawing_requests_text_edit(id), requests, "{kind:?}");
    }
}

#[test]
fn drawing_text_is_bounded_atomically_and_one_line_outside_the_text_owning_families() {
    let mut chart = settled_chart();
    let rectangle = chart
        .add_drawing(
            DrawingKind::Rectangle,
            0,
            vec![pt(2.0, 10.0), pt(7.0, 13.0)],
            Some(r#"{"text":"box"}"#),
        )
        .unwrap();
    // A patch with an oversized text applies nothing, not even its other fields.
    let before = chart.drawing(rectangle).unwrap().clone();
    let oversized = serde_json::json!({
        "color": "#ff0000",
        "text": "a".repeat(crate::MAX_DRAWING_TEXT_BYTES + 1),
    });
    assert!(!chart.drawing_apply_options(rectangle, &oversized.to_string()));
    assert_eq!(chart.drawing(rectangle).unwrap(), &before);
    let exact = serde_json::json!({ "text": "a".repeat(crate::MAX_DRAWING_TEXT_BYTES) });
    assert!(chart.drawing_apply_options(rectangle, &exact.to_string()));
    assert!(chart.undo_drawing());

    // An armed tool's template stays untouched by the same rejected patch.
    assert!(chart.set_drawing_tool(Some(DrawingKind::Rectangle), None, None));
    assert!(chart.drawing_tool_apply_options(&oversized.to_string()));
    let click = (x_at(&chart, 3.0), y_at(&chart, 11.0));
    let corner = (x_at(&chart, 6.0), y_at(&chart, 12.0));
    chart.drawing_tool_activate(click.0, click.1, DrawingModifiers::default());
    let created = chart
        .drawing_tool_activate(corner.0, corner.1, DrawingModifiers::default())
        .created
        .unwrap();
    let created = chart.drawing(created).unwrap();
    assert_eq!(created.text, "");
    assert_ne!(created.color, "#ff0000");

    // Live editor text clamps to the bound at a character boundary.
    assert!(chart.begin_drawing_text_edit(rectangle, false));
    assert!(chart.set_drawing_text_edit(&"€".repeat(40_000), usize::MAX));
    let clamped = &chart.drawing(rectangle).unwrap().text;
    assert_eq!(
        clamped.len(),
        21_845 * 3,
        "the longest prefix at a boundary"
    );
    assert!(clamped.len() <= crate::MAX_DRAWING_TEXT_BYTES);
    // The engine keeps a run label on one line: a run of CR/LF becomes one space.
    assert!(chart.set_drawing_text_edit("one\r\ntwo\n\nthree\rfour", usize::MAX));
    assert_eq!(chart.drawing(rectangle).unwrap().text, "one two three four");
    assert!(chart.cancel_drawing_text_edit());

    // A family box owns its lines.
    let comment = chart
        .add_drawing(DrawingKind::Comment, 0, vec![pt(6.0, 12.0)], None)
        .unwrap();
    assert!(chart.begin_drawing_text_edit(comment, false));
    assert!(chart.set_drawing_text_edit("one\ntwo", usize::MAX));
    assert_eq!(chart.drawing(comment).unwrap().text, "one\ntwo");
    assert!(chart.commit_drawing_text_edit());
}

#[test]
fn indexed_text_hit_matches_brute_force_for_rotated_boxed_and_full_extent_labels() {
    let mut chart = settled_chart();
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / f64::from(1_u32 << 31)
    };
    let kinds = [
        DrawingKind::TrendLine,
        DrawingKind::Ray,
        DrawingKind::ExtendedLine,
        DrawingKind::ArrowLine,
        DrawingKind::ParallelChannel,
        DrawingKind::Rectangle,
        DrawingKind::Ellipse,
        DrawingKind::HorizontalLine,
        DrawingKind::HorizontalRay,
        DrawingKind::VerticalLine,
        DrawingKind::FibRetracement,
        DrawingKind::AndrewsPitchfork,
        DrawingKind::Triangle,
    ];
    let texts = [
        "a",
        "medium label",
        "a very long label that runs far past the anchors of a short steep segment",
    ];
    let aligns = ["left", "center", "right"];
    let valigns = ["top", "middle", "bottom"];
    for index in 0..78 {
        let kind = kinds[index % kinds.len()];
        let steep = index % 3 == 0;
        let mut logical = 1.0 + next() * 7.0;
        let mut price = 10.0 + next() * 3.0;
        let points = (0..kind.anchor_count())
            .map(|_| {
                // A steep short drawing keeps its anchors within a hair of one bar.
                logical += if steep { 0.02 } else { next() * 3.0 - 1.0 };
                price += if steep {
                    next() - 0.5
                } else {
                    next() * 1.5 - 0.75
                };
                pt(logical, price)
            })
            .collect();
        chart
            .add_drawing(
                kind,
                0,
                points,
                Some(&format!(
                    r#"{{"text":"{}","text_h_align":"{}","text_v_align":"{}"}}"#,
                    texts[index % texts.len()],
                    aligns[(index / 3) % 3],
                    valigns[(index / 5) % 3],
                )),
            )
            .unwrap();
    }
    let mut hits = 0;
    for (from, to) in [
        (0.0, 9.0),
        (2.0, 6.0),
        (-3.0, 15.0),
        (4.4, 5.6),
        (7.0, 30.0),
    ] {
        chart.set_visible_logical_range(from, to);
        chart.build_frame();
        let mut probes = Vec::new();
        for gy in 0..25 {
            for gx in 0..40 {
                probes.push((f64::from(gx) * 20.0 + 3.0, f64::from(gy) * 20.0 + 5.0));
            }
        }
        // Walk every label's run, where a culled label would show as a disagreement.
        for id in chart
            .drawings()
            .iter()
            .map(|drawing| drawing.id)
            .collect::<Vec<_>>()
        {
            let Some(layout) = chart.drawing_text_edit_layout(id) else {
                continue;
            };
            let width = layout.rect[2] - layout.rect[0];
            for step in 0..=8 {
                let along = width * f64::from(step) / 8.0;
                probes.push((
                    layout.x + layout.angle.cos() * along,
                    layout.y + layout.angle.sin() * along,
                ));
            }
        }
        for (x, y) in probes {
            let indexed = chart.drawing_text_hit_at(x, y);
            assert_eq!(
                indexed,
                chart.drawing_text_hit_at_bruteforce(x, y),
                "{from}..{to} ({x}, {y})"
            );
            hits += usize::from(indexed.is_some());
        }
    }
    assert!(
        hits > 100,
        "the sweep must exercise real label hits: {hits}"
    );
}

#[test]
fn position_account_settings_are_atomic_and_survive_history_and_persistence() {
    let mut chart = settled_chart();
    let id = chart
        .add_drawing(
            DrawingKind::LongPosition,
            0,
            vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 11.0,
                },
                DrawingPoint {
                    logical: 6.0,
                    price: 12.0,
                },
                DrawingPoint {
                    logical: 2.0,
                    price: 10.0,
                },
            ],
            None,
        )
        .unwrap();
    assert!(chart.drawing_apply_options(
        id,
        r#"{"position_account_size":2000,"position_risk_percent":2}"#
    ));
    let updated = chart.drawing(id).unwrap().clone();
    assert_eq!(updated.position_account_size, 2000.0);
    assert_eq!(updated.position_risk_percent, 2.0);
    for patch in [
        r#"{"position_account_size":0,"color":"red"}"#,
        r#"{"position_risk_percent":101,"position_account_size":1000}"#,
    ] {
        assert!(!chart.drawing_apply_options(id, patch));
        assert_eq!(chart.drawing(id).unwrap(), &updated);
    }
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().position_account_size, 1000.0);
    assert!(chart.redo_drawing());
    let saved = chart.export_state_json().unwrap();
    let mut restored = settled_chart();
    restored.import_state_json(&saved).unwrap();
    assert_eq!(restored.drawing(id).unwrap().position_account_size, 2000.0);
    assert_eq!(restored.drawing(id).unwrap().position_risk_percent, 2.0);
    let mut invalid: serde_json::Value = serde_json::from_str(&saved).unwrap();
    invalid["drawings"][0]["style"]["position_risk_percent"] = serde_json::json!(-1.0);
    assert!(restored.import_state_json(&invalid.to_string()).is_err());
    assert_eq!(restored.drawing(id).unwrap().position_risk_percent, 2.0);
}

// --- measuring tools (price range, date range, date and price range, Shift-click measure) ---
//
// The three measuring tools are catalog entries of the Projection & Annotations family
// (`kinds/projection_annotations.rs`): they paint through the family range lowering (a `BandFill`
// area, crisp rules, arrows ended by the drawing's caps, and a statistics box), keep the drawing's
// own color in both directions, and print `+2.50  +25.00%  +250 ticks` / `10 bars  10h`. The
// transient Shift-click measure is a date-and-price range lowered through the same parts whose
// color alone follows its pull.

fn pane_texts(chart: &mut ChartEngine) -> Vec<String> {
    chart.build_frame().panes[0]
        .main
        .iter()
        .filter_map(|prim| match prim {
            Prim::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn market_down() -> Color {
    Color::parse_css(aeris_charts_core::style::MARKET_DOWN_CSS).unwrap()
}

fn primary() -> Color {
    Color::rgb(
        DEFAULT_PRIMARY_RGB.0,
        DEFAULT_PRIMARY_RGB.1,
        DEFAULT_PRIMARY_RGB.2,
    )
}

/// The measured-area wash: the tool color at 20% alpha, painted as one `BandFill`.
fn has_fill(chart: &mut ChartEngine, color: Color) -> bool {
    let wash = Color::rgba(color.r(), color.g(), color.b(), 51);
    chart.build_frame().panes[0]
        .main
        .iter()
        .any(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == wash))
}

/// Filled regions in exactly `color`: the arrowheads (caps) of the measured axes.
fn arrowheads(chart: &mut ChartEngine, color: Color) -> usize {
    chart.build_frame().panes[0]
        .main
        .iter()
        .filter(|prim| matches!(prim, Prim::BandFill { fill, .. } if *fill == color))
        .count()
}

fn add_measure(
    chart: &mut ChartEngine,
    kind: DrawingKind,
    from: (f64, f64),
    to: (f64, f64),
) -> DrawingId {
    chart
        .add_drawing(
            kind,
            0,
            vec![
                DrawingPoint {
                    logical: from.0,
                    price: from.1,
                },
                DrawingPoint {
                    logical: to.0,
                    price: to.1,
                },
            ],
            None,
        )
        .unwrap()
}

#[test]
fn measure_tools_have_stable_catalog_identity_and_defaults() {
    for (kind, wire, name) in [
        (DrawingKind::PriceRange, 130, "price_range"),
        (DrawingKind::DateRange, 131, "date_range"),
        (DrawingKind::DateAndPriceRange, 132, "date_and_price_range"),
    ] {
        assert_eq!(DrawingKind::from_u8(wire), Some(kind));
        assert_eq!(DrawingKind::from_name(name), Some(kind));
        assert_eq!((kind.to_u8(), kind.name()), (wire, name));
        assert_eq!(kind.anchor_count(), 2);
        assert!(kind.valid_point_count(2) && !kind.valid_point_count(3));
        assert!(kind.is_measure() && kind.spec().grid_snap);
        let drawing = Drawing::new(1, kind, 0, Vec::new());
        assert!(drawing.fill_enabled);
        assert_eq!(drawing.width, 1.0);
    }
    let mut unique = DRAWING_TOOL_SPECS
        .iter()
        .map(|spec| spec.wire_id)
        .collect::<Vec<_>>();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), DRAWING_TOOL_SPECS.len());
    // Only the measuring tools and the position tools snap their anchors to bars and ticks.
    for spec in DRAWING_TOOL_SPECS {
        let snaps = spec.kind.is_measure()
            || matches!(
                spec.kind,
                DrawingKind::LongPosition | DrawingKind::ShortPosition
            );
        assert_eq!(spec.grid_snap, snaps, "{:?}", spec.kind);
    }
}

#[test]
fn the_pre_merge_date_price_range_spelling_is_read_but_never_written() {
    let alias = "date_price_range";
    assert_eq!(
        DrawingKind::from_name(alias),
        Some(DrawingKind::DateAndPriceRange)
    );
    assert!(DRAWING_TOOL_SPECS.iter().all(|spec| spec.name != alias));
    assert_eq!(
        serde_json::from_str::<DrawingKind>(&format!("\"{alias}\"")).unwrap(),
        DrawingKind::DateAndPriceRange
    );
    assert_eq!(
        serde_json::to_value(DrawingKind::DateAndPriceRange).unwrap(),
        "date_and_price_range"
    );
}

#[test]
fn date_and_price_range_pulled_down_places_on_slots_and_ticks_and_reads_negative() {
    let mut chart = settled_chart();
    chart
        .set_instrument_metadata(crate::InstrumentMetadata {
            tick_size: Some(0.25),
            ..Default::default()
        })
        .unwrap();
    let spacing = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    assert!(chart.set_drawing_tool(Some(DrawingKind::DateAndPriceRange), None, None));
    let first = chart.drawing_tool_activate(
        x_at(&chart, 2.0) + spacing * 0.3,
        y_at(&chart, 12.1),
        DrawingModifiers::default(),
    );
    assert!(first.consumed && first.created.is_none());
    chart.drawing_tool_pointer_move(
        x_at(&chart, 7.0) - spacing * 0.3,
        y_at(&chart, 10.6),
        DrawingModifiers::default(),
        false,
    );
    // The live preview already shows the final statistics: ticks count on the instrument tick.
    let falling = "\u{2212}1.50  -12.50%  -6 ticks".to_string();
    let elapsed = "5 bars  5h".to_string();
    let texts = pane_texts(&mut chart);
    assert!(texts.contains(&falling), "{texts:?}");
    assert!(texts.contains(&elapsed), "{texts:?}");
    let id = chart
        .drawing_tool_activate(
            x_at(&chart, 7.0) - spacing * 0.3,
            y_at(&chart, 10.6),
            DrawingModifiers::default(),
        )
        .created
        .unwrap();
    let points = chart.drawing(id).unwrap().points.clone();
    assert_eq!(
        points,
        vec![
            DrawingPoint {
                logical: 2.0,
                price: 12.0
            },
            DrawingPoint {
                logical: 7.0,
                price: 10.5
            },
        ]
    );
    let texts = pane_texts(&mut chart);
    assert!(texts.contains(&falling), "{texts:?}");
    assert!(texts.contains(&elapsed), "{texts:?}");
    // A committed range keeps its own color in both directions: only the transient Shift-click
    // measure follows its pull.
    assert!(has_fill(&mut chart, primary()));
    assert!(!has_fill(&mut chart, market_down()));

    // Both arrows end in the drawing's cap and the label sits beyond the falling end level.
    assert_eq!(arrowheads(&mut chart, primary()), 2);
    assert_eq!(arrowheads(&mut chart, market_down()), 0);
    let end_y = y_at(&chart, 10.5);
    let frame = chart.build_frame();
    assert!(frame.panes[0].main.iter().any(|prim| matches!(prim,
        Prim::Text { text, y, .. } if *text == elapsed && f64::from(*y) > end_y)));
}

#[test]
fn measure_direction_follows_the_pull_for_every_tool() {
    let mut chart = settled_chart();
    // Pulled up: positive statistics, drawing color, label above the end level.
    let up = add_measure(
        &mut chart,
        DrawingKind::PriceRange,
        (2.0, 10.0),
        (6.0, 12.5),
    );
    let rising = "+2.50  +25.00%  +250 ticks".to_string();
    let texts = pane_texts(&mut chart);
    assert!(texts.contains(&rising), "{texts:?}");
    assert!(has_fill(&mut chart, primary()));
    let frame = chart.build_frame();
    let main = &frame.panes[0].main;
    let end_y = y_at(&chart, 12.5);
    assert!(main.iter().any(|prim| matches!(prim,
        Prim::Text { text, y, .. } if *text == rising && f64::from(*y) < end_y)));
    // The price tool frames its two levels with horizontal rules only.
    assert_eq!(
        main.iter()
            .filter(|prim| matches!(prim, Prim::HLine { color, .. } if *color == primary()))
            .count(),
        2
    );
    assert!(chart.remove_drawing(up));

    // Date range pulled backward in time: negative bars and elapsed time, vertical rules, and
    // still the drawing color (the direction is in the sign, not the paint).
    add_measure(&mut chart, DrawingKind::DateRange, (7.0, 11.0), (4.0, 12.0));
    let texts = pane_texts(&mut chart);
    assert!(texts.contains(&"-3 bars  -3h".to_string()), "{texts:?}");
    assert!(has_fill(&mut chart, primary()));
    assert!(!has_fill(&mut chart, market_down()));
    let frame = chart.build_frame();
    assert_eq!(
        frame.panes[0]
            .main
            .iter()
            .filter(|prim| matches!(prim, Prim::VLine { color, .. } if *color == primary()))
            .count(),
        2
    );
}

#[test]
fn measure_body_hits_inside_its_area_and_selection_exposes_both_anchors() {
    let mut chart = settled_chart();
    let id = add_measure(
        &mut chart,
        DrawingKind::DateAndPriceRange,
        (2.0, 10.5),
        (7.0, 12.5),
    );
    let spacing = x_at(&chart, 3.0) - x_at(&chart, 2.0);
    let inside = (x_at(&chart, 4.0) + spacing * 0.5, y_at(&chart, 11.5));
    let hit = chart.hit_test_drawing(inside.0, inside.1).unwrap();
    assert_eq!((hit.id, hit.part), (id, DrawingDragPart::Body));
    assert!(chart
        .hit_test_drawing(x_at(&chart, 8.0) + spacing * 0.5, y_at(&chart, 11.5))
        .is_none());
    chart.set_selected_drawing(Some(id));
    let end = chart
        .hit_test_drawing(x_at(&chart, 7.0), y_at(&chart, 12.5))
        .unwrap();
    assert_eq!(end.part, DrawingDragPart::Anchor(1));

    // Anchor drags stay on whole bars and price ticks (scale min_move 0.01 here).
    assert!(chart.drawing_drag_start_at(x_at(&chart, 7.0), y_at(&chart, 12.5)));
    chart.drawing_drag_to(
        x_at(&chart, 8.0) + spacing * 0.2,
        y_at(&chart, 12.0) + 0.37,
        DrawingModifiers::default(),
    );
    chart.drawing_drag_end();
    let end = chart.drawing(id).unwrap().points[1];
    assert_eq!(end.logical, 8.0);
    assert!((end.price * 100.0 - (end.price * 100.0).round()).abs() < 1e-6);
}

#[test]
fn measure_persists_and_its_axis_views_follow_selection() {
    let mut chart = settled_chart();
    let id = add_measure(
        &mut chart,
        DrawingKind::DateAndPriceRange,
        (2.0, 12.0),
        (6.0, 10.0),
    );
    chart.axis_w = 80.0;
    chart.build_frame();
    // Axis tags carry the drawing's own color as their background.
    let tags = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |text, _bold| text.len() as f64 * 7.0,
                |text, _bold| text.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .filter(|label| {
                label
                    .background
                    .is_some_and(|background| background.4 == primary())
            })
            .count()
    };
    let before = tags(&mut chart);
    chart.set_selected_drawing(Some(id));
    // Two price tags plus two time tags while the range is selected.
    assert_eq!(tags(&mut chart) - before, 4);

    assert!(chart.drawing_apply_options(id, r#"{"fill_enabled":false}"#));
    let saved = chart.export_state_json().unwrap();
    // The canonical name is written; the pre-merge spelling never is.
    assert!(saved.contains("\"date_and_price_range\""));
    assert!(!saved.contains("date_price_range"));
    let mut restored = settled_chart();
    restored.import_state_json(&saved).unwrap();
    let drawing = restored.drawing(id).unwrap();
    assert_eq!(drawing.kind, DrawingKind::DateAndPriceRange);
    assert_eq!(drawing.points, chart.drawing(id).unwrap().points);
    assert!(!drawing.fill_enabled);

    // Documents written by upstream builds name the earlier spelling and still import.
    let legacy = saved.replace("\"date_and_price_range\"", "\"date_price_range\"");
    let mut migrated = settled_chart();
    migrated.import_state_json(&legacy).unwrap();
    let drawing = migrated.drawing(id).unwrap();
    assert_eq!(drawing.kind, DrawingKind::DateAndPriceRange);
    assert_eq!(drawing.points, chart.drawing(id).unwrap().points);
    assert!(!migrated
        .export_state_json()
        .unwrap()
        .contains("date_price_range"));
}

#[test]
fn shift_measure_drag_freezes_on_release_and_the_next_press_dismisses_it() {
    let mut chart = settled_chart();
    let start = (x_at(&chart, 3.0), y_at(&chart, 11.0));
    let end = (x_at(&chart, 6.0), y_at(&chart, 12.5));
    let none = DrawingModifiers::default();
    // Without Shift (or an existing measure) the press belongs to the host's other gestures.
    assert!(!chart.measure_pointer_down(start.0, start.1, false, none));
    assert!(chart.measure_pointer_down(start.0, start.1, true, none));
    assert!(chart.measure_following());
    assert!(chart.measure_pointer_move(end.0, end.1, none));
    assert!(chart.measure_pointer_up(end.0, end.1, none));
    assert!(chart.measure_active() && !chart.measure_following());
    assert_eq!(
        chart.measure_points().unwrap(),
        [
            DrawingPoint {
                logical: 3.0,
                price: 11.0
            },
            DrawingPoint {
                logical: 6.0,
                price: 12.5
            },
        ]
    );
    // Frozen: moves no longer change it.
    assert!(!chart.measure_pointer_move(start.0, start.1, none));
    let texts = pane_texts(&mut chart);
    assert!(
        texts.contains(&"+1.50  +13.64%  +150 ticks".to_string()),
        "{texts:?}"
    );
    assert!(texts.contains(&"3 bars  3h".to_string()));
    // A rising pull paints in the drawing default color.
    assert!(has_fill(&mut chart, primary()));
    // Transient: never a drawing, history entry, or persisted object.
    assert!(chart.drawings().is_empty());
    assert!(!chart.can_undo_drawing());
    let saved = chart.export_state_json().unwrap();
    assert!(!saved.contains("date_and_price_range") && !saved.contains("date_price_range"));

    assert!(chart.measure_pointer_down(end.0, end.1, false, none));
    assert!(!chart.measure_active());
    assert!(!pane_texts(&mut chart).contains(&"3 bars  3h".to_string()));
}

#[test]
fn shift_measure_click_move_click_and_cancellation() {
    let mut chart = settled_chart();
    let none = DrawingModifiers::default();
    let start = (x_at(&chart, 6.0), y_at(&chart, 12.0));
    assert!(chart.measure_pointer_down(start.0, start.1, true, none));
    // A release inside the click slop keeps following until the next press.
    assert!(!chart.measure_pointer_up(start.0 + 2.0, start.1 + 1.0, none));
    assert!(chart.measure_following());
    assert!(chart.measure_pointer_move(x_at(&chart, 2.0), y_at(&chart, 10.0), none));
    assert!(chart.measure_pointer_down(x_at(&chart, 2.0), y_at(&chart, 10.0), false, none));
    assert!(!chart.measure_following());
    assert!(!chart.measure_pointer_up(x_at(&chart, 2.0), y_at(&chart, 10.0), none));
    let texts = pane_texts(&mut chart);
    assert!(
        texts.contains(&"\u{2212}2.00  -16.67%  -200 ticks".to_string()),
        "{texts:?}"
    );
    assert!(texts.contains(&"-4 bars  -4h".to_string()), "{texts:?}");
    // A falling pull paints in the market-down color, not the drawing default.
    assert!(has_fill(&mut chart, market_down()));
    assert!(!has_fill(&mut chart, primary()));

    // A pointer beyond the pane clamps into it instead of dropping the measure.
    assert!(chart.cancel_measure());
    assert!(chart.measure_pointer_down(start.0, start.1, true, none));
    chart.measure_pointer_move(start.0, -500.0, none);
    let top_price = chart.measure_points().unwrap()[1].price;
    assert!(top_price.is_finite() && top_price > 12.0);

    // Escape (the drawing-tool cancel) dismisses it; arming a tool replaces it and owns presses.
    chart.cancel_drawing_tool();
    assert!(!chart.measure_active());
    assert!(chart.measure_pointer_down(start.0, start.1, true, none));
    assert!(chart.set_drawing_tool(Some(DrawingKind::TrendLine), None, None));
    assert!(!chart.measure_active());
    assert!(!chart.measure_pointer_down(start.0, start.1, true, none));
}

#[test]
fn shift_measure_color_follows_its_pull_and_flips_with_the_end_anchor() {
    let mut chart = settled_chart();
    let none = DrawingModifiers::default();
    let start = (x_at(&chart, 4.0), y_at(&chart, 11.0));
    assert!(chart.measure_pointer_down(start.0, start.1, true, none));
    // Pulled up, then down, then up again: the session recolors on each flip of the pull.
    for (price, down) in [(12.0, false), (10.0, true), (13.0, false)] {
        assert!(chart.measure_pointer_move(x_at(&chart, 7.0), y_at(&chart, price), none));
        assert_eq!(has_fill(&mut chart, market_down()), down, "{price}");
        assert_eq!(has_fill(&mut chart, primary()), !down, "{price}");
    }
    // A committed range keeps its own color whatever the transient measure does.
    chart.cancel_measure();
    add_measure(
        &mut chart,
        DrawingKind::PriceRange,
        (2.0, 12.0),
        (6.0, 10.0),
    );
    assert!(has_fill(&mut chart, primary()));
    assert!(!has_fill(&mut chart, market_down()));
}

#[test]
fn grid_snapped_ranges_nudge_by_at_least_one_bar_and_one_tick() {
    for kind in [
        DrawingKind::PriceRange,
        DrawingKind::DateRange,
        DrawingKind::DateAndPriceRange,
    ] {
        let mut chart = settled_chart();
        let id = add_measure(&mut chart, kind, (2.0, 10.5), (7.0, 12.5));
        chart.set_selected_drawing(Some(id));
        let before = chart.drawing(id).unwrap().points.clone();
        // A one-pixel key step is far below a bar or a tick, yet a handle moves one whole bar.
        assert!(chart.nudge_selected_drawing(1.0, 0.0, Some(0)), "{kind:?}");
        let points = chart.drawing(id).unwrap().points.clone();
        assert_eq!(points[0].logical, 3.0, "{kind:?}");
        assert_eq!(points[1], before[1], "{kind:?}");
        assert!(chart.undo_drawing(), "{kind:?}");
        assert_eq!(chart.drawing(id).unwrap().points, before, "{kind:?}");
        // The body moves rigidly by whole bars.
        assert!(chart.nudge_selected_drawing(-1.0, 0.0, None), "{kind:?}");
        let points = chart.drawing(id).unwrap().points.clone();
        assert_eq!(
            (points[0].logical, points[1].logical),
            (1.0, 6.0),
            "{kind:?}"
        );
        assert!(chart.undo_drawing(), "{kind:?}");
        // Up is one price tick (0.01 on this scale) or more, and lands on the tick grid.
        assert!(chart.nudge_selected_drawing(0.0, -1.0, Some(1)), "{kind:?}");
        let moved = chart.drawing(id).unwrap().points[1];
        assert!(moved.price > before[1].price, "{kind:?}");
        assert!((moved.price * 100.0 - (moved.price * 100.0).round()).abs() < 1e-6);
        assert!(chart.undo_drawing(), "{kind:?}");
        assert_eq!(chart.drawing(id).unwrap().points, before, "{kind:?}");
    }
}

/// The three ways a magnet can be on (the weak chart mode, the strong chart mode, and Ctrl
/// toggling an off chart into a strong one) with the modifiers each one is driven by.
fn magnet_cases() -> [(crate::DrawingMagnetMode, DrawingModifiers, &'static str); 3] {
    use crate::DrawingMagnetMode::{Off, Strong, Weak};
    [
        (Weak, NONE, "weak chart magnet"),
        (Strong, NONE, "strong chart magnet"),
        (Off, MAGNET, "ctrl on an off chart"),
    ]
}

/// A range chart with room on both sides of the data (bars 2 to 11 are on screen), magnet `mode`
/// on the chart.
fn range_chart(mode: crate::DrawingMagnetMode) -> ChartEngine {
    let mut chart = settled_chart();
    chart.set_bar_spacing(40.0);
    chart.set_right_offset(4.0);
    chart.build_frame();
    chart.set_drawing_magnet_mode(mode);
    chart
}

/// Pointer ends for the magnet tests: off-bar slots, halfway between two prices (farther from
/// every candle than the weak magnet reaches), the last one beyond the last bar where no candle
/// exists for any magnet to choose.
fn off_bar_pointers(chart: &ChartEngine) -> Vec<(f64, f64)> {
    let y = y_at(chart, 11.5);
    assert!((y - y_at(chart, 11.0)).abs() > DRAWING_WEAK_MAGNET_DISTANCE);
    assert!((y - y_at(chart, 12.0)).abs() > DRAWING_WEAK_MAGNET_DISTANCE);
    let pointers =
        [2.3, 6.3, 11.3].map(|logical| (chart.time_scale.logical_to_coordinate(logical), y));
    assert!(pointers.iter().all(|&(x, _)| x < chart.pane_w));
    pointers.to_vec()
}

/// An anchor of a grid-snapped tool sits on the crosshair's whole slot under the pointer and on
/// the price tick grid, whether or not a magnet was on.
fn assert_on_the_slot_grid(chart: &ChartEngine, point: DrawingPoint, x: f64, context: &str) {
    assert_eq!(
        point.logical,
        chart.snapped_crosshair_index(x) as f64,
        "{context}: whole slot"
    );
    assert!(
        (point.price * 100.0 - (point.price * 100.0).round()).abs() < 1e-6,
        "{context}: price tick ({})",
        point.price
    );
}

#[test]
fn magnet_that_chooses_no_candle_still_places_range_anchors_on_whole_bars() {
    for (mode, modifiers, case) in magnet_cases() {
        for kind in [
            DrawingKind::PriceRange,
            DrawingKind::DateRange,
            DrawingKind::DateAndPriceRange,
        ] {
            let mut chart = range_chart(mode);
            let pointers = off_bar_pointers(&chart);
            for &(end_x, end_y) in &pointers[1..] {
                let (start_x, start_y) = pointers[0];
                let context = format!("{kind:?} {case} end at x={end_x:.1}");
                assert!(chart.set_drawing_tool(Some(kind), None, None));
                let first = chart.drawing_tool_activate(start_x, start_y, modifiers);
                assert!(first.consumed && first.created.is_none(), "{context}");
                // The live preview already follows the same grid as the committed anchor.
                chart.drawing_tool_pointer_move(end_x, end_y, modifiers, false);
                let preview = chart.pending_drawing().unwrap().preview.unwrap();
                assert_on_the_slot_grid(&chart, preview, end_x, &format!("{context} preview"));
                let id = chart
                    .drawing_tool_activate(end_x, end_y, modifiers)
                    .created
                    .unwrap();
                let points = chart.drawing(id).unwrap().points.clone();
                assert_on_the_slot_grid(&chart, points[0], start_x, &format!("{context} start"));
                assert_on_the_slot_grid(&chart, points[1], end_x, &format!("{context} end"));
                assert!(chart.remove_drawing(id));
            }
        }
    }
}

#[test]
fn magnet_that_chooses_no_candle_still_drags_range_anchors_onto_whole_bars() {
    for (mode, modifiers, case) in magnet_cases() {
        for kind in [
            DrawingKind::PriceRange,
            DrawingKind::DateRange,
            DrawingKind::DateAndPriceRange,
        ] {
            for handle in 0..2 {
                for target in 0..3 {
                    // A fresh chart per drag: a drag the magnet leaves where it started records
                    // no history entry to undo.
                    let mut chart = range_chart(mode);
                    let (x, y) = off_bar_pointers(&chart)[target];
                    let context = format!("{kind:?} {case} handle {handle} to x={x:.1}");
                    let id = add_measure(&mut chart, kind, (2.0, 11.0), (6.0, 12.0));
                    chart.set_selected_drawing(Some(id));
                    chart.build_frame();
                    let (hx, hy) = chart.drawing_point_to_coordinate(id, handle).unwrap();
                    let hit = chart.hit_test_drawing(hx, hy).unwrap();
                    assert_eq!(
                        (hit.id, hit.part),
                        (id, DrawingDragPart::Anchor(handle)),
                        "{context}"
                    );
                    assert!(chart.drawing_drag_start_at(hx, hy), "{context}");
                    chart.drawing_drag_to(x, y, modifiers);
                    chart.drawing_drag_end();
                    let points = chart.drawing(id).unwrap().points.clone();
                    assert_on_the_slot_grid(&chart, points[handle], x, &context);
                }
            }
        }
    }
}

#[test]
fn magnet_that_chooses_no_candle_still_places_the_shift_measure_on_whole_bars() {
    for (mode, modifiers, case) in magnet_cases() {
        let mut chart = range_chart(mode);
        let pointers = off_bar_pointers(&chart);
        for &(end_x, end_y) in &pointers[1..] {
            let (start_x, start_y) = pointers[0];
            let context = format!("{case} end at x={end_x:.1}");
            assert!(chart.measure_pointer_down(start_x, start_y, true, modifiers));
            assert!(
                chart.measure_pointer_move(end_x, end_y, modifiers),
                "{context}"
            );
            let [start, end] = chart.measure_points().unwrap();
            assert_on_the_slot_grid(&chart, start, start_x, &format!("{context} start"));
            assert_on_the_slot_grid(&chart, end, end_x, &format!("{context} end"));
            assert!(chart.cancel_measure());
        }
    }
}

#[test]
fn magnet_that_chooses_a_candle_keeps_its_bar_and_price_for_range_anchors() {
    // A strong magnet next to a candle still wins: the anchor takes the candle's own price
    // (not the pointer's) on that candle's slot.
    let mut chart = range_chart(crate::DrawingMagnetMode::Strong);
    let at = |chart: &ChartEngine, logical: f64, price: f64| {
        (
            chart.time_scale.logical_to_coordinate(logical),
            y_at(chart, price),
        )
    };
    let (x, y) = at(&chart, 6.2, 12.6);
    assert!(chart.set_drawing_tool(Some(DrawingKind::PriceRange), None, None));
    assert!(chart.drawing_tool_activate(x, y, NONE).created.is_none());
    let (x, y) = at(&chart, 2.2, 10.6);
    chart.drawing_tool_pointer_move(x, y, NONE, false);
    let id = chart.drawing_tool_activate(x, y, NONE).created.unwrap();
    let points = chart.drawing(id).unwrap().points.clone();
    assert_eq!(
        points,
        vec![
            DrawingPoint {
                logical: 6.0,
                price: 13.0
            },
            DrawingPoint {
                logical: 2.0,
                price: 11.0
            },
        ]
    );
}

#[test]
fn range_ticks_count_on_the_instrument_tick_and_the_price_band_ladder() {
    let mut chart = settled_chart();
    chart
        .set_instrument_metadata(crate::InstrumentMetadata {
            tick_size: Some(0.25),
            ..Default::default()
        })
        .unwrap();
    // The instrument tick, not the 0.01 display tick: 1.50 is six ticks.
    let range = add_measure(
        &mut chart,
        DrawingKind::PriceRange,
        (2.0, 10.0),
        (6.0, 11.5),
    );
    let texts = pane_texts(&mut chart);
    assert!(
        texts.contains(&"+1.50  +15.00%  +6 ticks".to_string()),
        "{texts:?}"
    );
    assert!(chart.remove_drawing(range));

    // A price-band ladder is the scale's single tick source: 100 ticks of 0.01 below 10, then
    // 0.02 ticks, so 9.00 to 11.00 is 150 ticks whatever the instrument tick says.
    assert!(chart.series_apply_price_format_json(
        0,
        r#"{"type":"price","tick_ladder":[{"from":0,"min_move":0.01},{"from":10,"min_move":0.02}]}"#
    ));
    chart.build_frame();
    let range = add_measure(&mut chart, DrawingKind::PriceRange, (2.0, 9.0), (6.0, 11.0));
    let texts = pane_texts(&mut chart);
    assert!(
        texts.iter().any(|text| text.ends_with("+150 ticks")),
        "{texts:?}"
    );
    // Free anchors sit off the ladder grid: ticks count between the nearest grid prices
    // (9.00 to 11.02 is 151 ticks) instead of dropping the metric.
    assert!(chart.remove_drawing(range));
    add_measure(
        &mut chart,
        DrawingKind::PriceRange,
        (2.0, 9.0),
        (6.0, 11.013),
    );
    let texts = pane_texts(&mut chart);
    assert!(
        texts.iter().any(|text| text.ends_with("+151 ticks")),
        "{texts:?}"
    );
}

#[test]
fn measure_elapsed_time_follows_the_anchor_time_identity_not_the_display_projection() {
    let mut chart = settled_chart();
    add_measure(
        &mut chart,
        DrawingKind::DateRange,
        (6.0, 11.0),
        (12.0, 11.0),
    );
    // Slot 12 is beyond the data: the elapsed time extrapolates with the prevailing bar interval,
    // and a display-only time projection never changes what the anchors measure.
    assert!(pane_texts(&mut chart).contains(&"6 bars  6h".to_string()));
    assert!(chart.set_future_time_projection(Some(3600), 10));
    let texts = pane_texts(&mut chart);
    assert!(texts.contains(&"6 bars  6h".to_string()), "{texts:?}");
    // A reload at a 2h cadence re-spaces the bars under the anchors, which keep their time
    // identity: the bar count follows the new spacing and the elapsed time stays 6h, and the
    // label refreshes without any pointer or option change.
    let times = (0..10).map(|i| (i * 7200) as f64).collect::<Vec<_>>();
    let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
    chart
        .set_series_data(0, &times, &values, &values, &values, &values)
        .unwrap();
    let texts = pane_texts(&mut chart);
    assert!(texts.contains(&"3 bars  6h".to_string()), "{texts:?}");
}

#[test]
fn measure_axis_time_tags_extrapolate_beyond_the_data_like_the_statistics() {
    let mut chart = settled_chart();
    // Leave room after the last bar so slot 12, beyond the ten bars, is inside the plot.
    chart.set_right_offset(6.0);
    chart.build_frame();
    assert!((0.0..=chart.pane_w).contains(&x_at(&chart, 6.0)));
    assert!((0.0..=chart.pane_w).contains(&x_at(&chart, 12.0)));
    let id = add_measure(
        &mut chart,
        DrawingKind::DateRange,
        (6.0, 11.0),
        (12.0, 11.0),
    );
    chart.axis_w = 80.0;
    chart.build_frame();
    let tags = |chart: &mut ChartEngine| {
        chart
            .build_axis_frame(
                80.0,
                |text, _bold| text.len() as f64 * 7.0,
                |text, _bold| text.len() as f64 * 6.0,
            )
            .labels
            .into_iter()
            .filter(|label| {
                label
                    .background
                    .is_some_and(|background| background.4 == primary())
            })
            .count()
    };
    let before = tags(&mut chart);
    // Slot 12 is beyond the data: the statistics print the extrapolated "6 bars  6h", so the
    // time axis keeps a tag for that anchor too instead of silently dropping it.
    assert!(pane_texts(&mut chart).contains(&"6 bars  6h".to_string()));
    chart.set_selected_drawing(Some(id));
    assert_eq!(tags(&mut chart) - before, 2);
}

#[test]
fn measure_statistics_never_print_a_signed_zero() {
    // A change that rounds to zero is unsigned in both the price and the percentage, whatever
    // its direction.
    for end in [10.9999, 11.0001] {
        let mut chart = settled_chart();
        add_measure(&mut chart, DrawingKind::PriceRange, (2.0, 11.0), (6.0, end));
        let texts = pane_texts(&mut chart);
        assert!(
            texts.iter().any(|text| text.starts_with("0.00  0.00%")),
            "{end}: {texts:?}"
        );
    }
}

#[test]
fn measure_prices_honour_the_instrument_precision() {
    let mut chart = settled_chart();
    chart
        .set_instrument_metadata(crate::InstrumentMetadata {
            tick_size: Some(0.0001),
            price_precision: Some(4),
            ..Default::default()
        })
        .unwrap();
    add_measure(
        &mut chart,
        DrawingKind::PriceRange,
        (2.0, 10.0),
        (6.0, 10.0123),
    );
    // The same four decimals a Long Position on this chart prints, not the factory two.
    let texts = pane_texts(&mut chart);
    assert!(
        texts.iter().any(|text| text.starts_with("+0.0123  ")),
        "{texts:?}"
    );
}

#[test]
fn measure_label_keeps_an_offscreen_area_in_the_viewport_candidates() {
    let mut chart = settled_chart();
    // More than 20 drawings switches frame construction to indexed viewport culling.
    for index in 0..24 {
        add_measure(
            &mut chart,
            DrawingKind::PriceRange,
            (1.0 + index as f64 * 0.1, 10.5),
            (2.0, 11.0),
        );
    }
    // The area ends 34 px above the pane, beyond the touch hit padding, while the two-line
    // statistics box beyond that end still reaches into the pane.
    let above = chart.series_coordinate_to_price(0, -90.0).unwrap();
    let edge = chart.series_coordinate_to_price(0, -34.0).unwrap();
    // A fall that ends just above the pane: its box paints below the end, partly inside the pane.
    add_measure(
        &mut chart,
        DrawingKind::DateAndPriceRange,
        (4.0, above),
        (6.0, edge),
    );
    let falling = pane_texts(&mut chart)
        .into_iter()
        .filter(|text| text.starts_with('\u{2212}'))
        .count();
    assert_eq!(falling, 1);
}

#[test]
fn drawing_revision_advances_on_every_committed_path_and_not_on_hover_selection_drag_or_typing() {
    // Hosts persist when this one counter changes (the drawing sync revision). A cell's
    // accepted sync payload replaces its drawings, so it counts too.
    let mut source = settled_chart();
    add_trend(&mut source);
    let payload = source.drawing_sync_payload_json("cell-b").unwrap();
    let mut chart = settled_chart();
    let mut last = chart.drawing_revision();
    fn advanced(chart: &ChartEngine, last: &mut u64, what: &str) {
        let now = chart.drawing_revision();
        assert!(
            now > *last,
            "{what} advances the revision ({last} -> {now})"
        );
        *last = now;
    }
    assert!(chart.apply_drawing_sync_payload_json(&payload));
    advanced(&chart, &mut last, "an accepted sync payload");
    let id = chart.drawings()[0].id;
    // Same payload again: stale, rejected, no revision.
    assert!(!chart.apply_drawing_sync_payload_json(&payload));
    assert_eq!(chart.drawing_revision(), last);

    // Hover, selection, an unfinished drag, and live typing are not committed edits.
    let anchor = (x_at(&chart, 2.0), y_at(&chart, 10.5));
    chart.update_drawing_hover(anchor.0, anchor.1);
    chart.set_selected_drawing(Some(id));
    assert!(chart.drawing_drag_start_at(anchor.0, anchor.1));
    chart.drawing_drag_to(
        anchor.0 + 30.0,
        anchor.1 - 20.0,
        DrawingModifiers::default(),
    );
    assert_eq!(chart.drawing_revision(), last, "an unfinished drag");
    chart.drawing_drag_end();
    advanced(&chart, &mut last, "a finished anchor drag");

    chart.build_frame();
    assert!(chart.begin_drawing_text_edit(id, true));
    assert!(chart.drawing_text_edit_insert("note"));
    assert_eq!(chart.drawing_revision(), last, "live typing");
    assert!(chart.commit_drawing_text_edit());
    advanced(&chart, &mut last, "a text commit");

    assert!(chart.drawing_apply_options(id, r##"{"color":"#123456"}"##));
    advanced(&chart, &mut last, "a style change");
    assert!(chart.undo_drawing());
    advanced(&chart, &mut last, "undo");
    assert!(chart.redo_drawing());
    advanced(&chart, &mut last, "redo");
    assert!(chart.set_drawing_price_basis(Some("adjusted")).is_ok());
    advanced(&chart, &mut last, "a price-basis label");
    let rescaled = chart
        .rescale_drawing_prices(
            &[crate::DrawingPriceSegment {
                from_time: None,
                to_time: None,
                factor: 2.0,
            }],
            None,
        )
        .unwrap();
    assert_eq!(rescaled, 1);
    advanced(&chart, &mut last, "a price rescale");
    assert!(chart.remove_drawing(id));
    advanced(&chart, &mut last, "a delete");
    let created = add_trend(&mut chart);
    advanced(&chart, &mut last, "a create");
    assert!(chart.remove_drawing(created));
}
