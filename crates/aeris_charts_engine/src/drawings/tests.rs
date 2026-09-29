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
/// executor as solid dash runs lowered in the frame, like family strokes and series lines: the
/// WebGPU stroker ignores `Polyline.style`, so a styled polyline would paint solid there while
/// Canvas2D dashes it. Solid strokes keep their single polyline, and a dashed line reaching far
/// past the pane is clipped before it is split, so its frame work stays bounded.
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
    assert!(
        main.iter()
            .any(|p| matches!(p, Prim::RectFrame { color, .. } if *color == drawing_color)),
        "the default border is the solid frame"
    );

    // Dotted/dashed borders: the frame becomes four crisp line prims in the drawing color.
    assert!(chart.drawing_apply_options(id, r#"{"style":"dotted"}"#));
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
    chart.build_axis_primitives_into(&committed_axis, &mut axis_primitives, |_| 0.0);
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
    // Ctrl held (the public reference's temporary magnet): the line snaps to the bar's high.
    chart.crosshair_ohlc_magnet = true;
    assert_eq!(
        crosshair_hline_y(&mut chart),
        Some(y_at(&chart, 12.0).round() as i32)
    );
    let crosshair_y = crosshair_hline_y(&mut chart).unwrap();
    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
    let drawing_id = chart.drawing_create_click(x, y_free, MAGNET) as DrawingId;
    let (_, drawing_y) = chart
        .drawing_to_px(0, chart.drawing(drawing_id).unwrap().points[0])
        .unwrap();
    assert_eq!(
        drawing_y.round() as i32,
        crosshair_y,
        "crosshair and drawing magnets must resolve the same pixel-space OHLC candidate"
    );
    // Released: raw again (the configured mode is untouched).
    chart.crosshair_ohlc_magnet = false;
    assert_eq!(crosshair_hline_y(&mut chart), Some(y_free.round() as i32));
    // Hidden mode stays hidden even with the flag set.
    chart.crosshair_ohlc_magnet = true;
    chart.crosshair_mode = aeris_charts_core::model::magnet::CrosshairMode::Hidden;
    assert_eq!(crosshair_hline_y(&mut chart), None);
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
    chart.crosshair_ohlc_magnet = true;
    assert_eq!(
        crosshair_hline_y(&mut chart),
        Some(rendered_y.round() as i32),
        "an area series exposes only its rendered close/value to the OHLC magnet"
    );

    assert!(chart.drawing_create_begin(DrawingKind::HorizontalLine, None));
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
    assert!(chart.begin_drawing_text_edit(id));
    assert_eq!(chart.editing_drawing(), Some(id));
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "live"));
    assert!(chart.end_drawing_text_edit(true));
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
    assert!(chart.begin_drawing_text_edit(id));
    for text in ["ab", "abc", "abcd"] {
        assert!(chart.set_drawing_edit_text(text));
    }
    let (texts, _) = text_prims(&mut chart);
    assert!(texts.iter().any(|(t, _)| t == "abcd"), "live text paints");
    assert!(chart.end_drawing_text_edit(true));
    assert!(chart.undo_drawing());
    assert_eq!(chart.drawing(id).unwrap().text, "a", "one step per edit");
    assert!(chart.undo_drawing());
    assert!(chart.drawing(id).is_none(), "the next step is the creation");
    assert!(chart.redo_drawing() && chart.redo_drawing());
    assert_eq!(chart.drawing(id).unwrap().text, "abcd");

    assert!(chart.drawing_apply_options(id, r#"{"locked":true}"#));
    assert!(!chart.drawing_text_editable(id));
    assert!(!chart.begin_drawing_text_edit(id));
    assert_eq!(chart.editing_drawing(), None);
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
