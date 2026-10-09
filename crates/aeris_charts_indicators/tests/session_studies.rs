use aeris_charts_indicators::{
    IndicatorInput, PreviousPeriod, SessionSource, SessionSpan, SessionStudy, SessionStudyPoint,
    SessionStudyState, session_study,
};

fn input<'a>(
    times: &'a [i64],
    highs: &'a [f64],
    lows: &'a [f64],
    closes: &'a [f64],
) -> IndicatorInput<'a> {
    IndicatorInput {
        times,
        open: closes,
        high: highs,
        low: lows,
        close: closes,
        volume: &[],
        amount: &[],
    }
}

fn point(high: f64, low: f64, close: Option<f64>) -> SessionStudyPoint {
    SessionStudyPoint {
        high: Some(high),
        low: Some(low),
        close,
    }
}

#[test]
fn incremental_utc_append_tip_and_checkpoint_repairs_match_full() {
    let times: Vec<_> = (0..2_175).map(|row| row as i64 * 3_600).collect();
    let mut highs: Vec<_> = (0..times.len())
        .map(|row| 100. + (row % 31) as f64)
        .collect();
    let mut lows: Vec<_> = (0..times.len())
        .map(|row| 70. - (row % 13) as f64)
        .collect();
    let mut closes: Vec<_> = (0..times.len())
        .map(|row| 90. + (row % 17) as f64)
        .collect();
    highs[1023] = f64::NAN;
    lows[1023] = f64::NAN;
    closes[1023] = f64::NAN;
    for kind in [
        SessionStudy::SessionLevels,
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Week),
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Month),
        SessionStudy::OpeningRange {
            duration_seconds: 12 * 3_600,
        },
    ] {
        let mut state = SessionStudyState::new(kind);
        for (previous, end) in [
            (0, 1),
            (1, 1023),
            (1023, 1024),
            (1024, 1025),
            (1025, 2048),
            (2048, times.len()),
        ] {
            let bars = input(&times[..end], &highs[..end], &lows[..end], &closes[..end]);
            state.update(bars, SessionSource::Utc, previous);
            assert_eq!(
                state.outputs(),
                session_study(bars, SessionSource::Utc, kind),
                "append {end} {kind:?}"
            );
        }
        for row in [times.len() - 1, 1024, 1023, 79, 2050] {
            highs[row] = 300. + row as f64;
            lows[row] = -10. - row as f64;
            closes[row] = 200. + row as f64;
            let bars = input(&times, &highs, &lows, &closes);
            state.update(bars, SessionSource::Utc, row);
            assert_eq!(
                state.outputs(),
                session_study(bars, SessionSource::Utc, kind),
                "repair {row} {kind:?}"
            );
        }
        // Pure truncation must retain the correct before-tip state.
        let end = 2049;
        state.update(
            input(&times[..end], &highs[..end], &lows[..end], &closes[..end]),
            SessionSource::Utc,
            end,
        );
        highs[end - 1] += 500.;
        let bars = input(&times[..end], &highs[..end], &lows[..end], &closes[..end]);
        state.update(bars, SessionSource::Utc, end - 1);
        assert_eq!(
            state.outputs(),
            session_study(bars, SessionSource::Utc, kind),
            "truncation/tip {kind:?}"
        );
    }
}

#[test]
fn incremental_host_cursor_replays_across_merged_spans_and_gaps() {
    const DAY: i64 = 86_400;
    let spans = [
        SessionSpan {
            start: 0,
            end: 5 * DAY,
            session_id: 1,
        },
        SessionSpan {
            start: 5 * DAY,
            end: 12 * DAY,
            session_id: 1,
        },
        SessionSpan {
            start: 14 * DAY,
            end: 28 * DAY,
            session_id: 2,
        },
        SessionSpan {
            start: 28 * DAY,
            end: 65 * DAY,
            session_id: 3,
        },
        SessionSpan {
            start: 67 * DAY,
            end: 120 * DAY,
            session_id: 4,
        },
    ];
    let times: Vec<_> = (0..2_600).map(|row| row as i64 * 3_600).collect();
    let mut highs: Vec<_> = (0..times.len())
        .map(|row| 100. + (row % 41) as f64)
        .collect();
    let lows: Vec<_> = (0..times.len())
        .map(|row| 50. - (row % 19) as f64)
        .collect();
    let closes: Vec<_> = (0..times.len())
        .map(|row| 80. + (row % 11) as f64)
        .collect();
    let source = SessionSource::Host(&spans);
    for kind in [
        SessionStudy::SessionLevels,
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Week),
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Month),
        SessionStudy::OpeningRange {
            duration_seconds: 7 * DAY,
        },
    ] {
        let mut state = SessionStudyState::new(kind);
        for (from, end) in [(0, 1024), (1024, 2048), (2048, times.len())] {
            let bars = input(&times[..end], &highs[..end], &lows[..end], &closes[..end]);
            state.update(bars, source, from);
            assert_eq!(
                state.outputs(),
                session_study(bars, source, kind),
                "append {end} {kind:?}"
            );
        }
        for row in [2_599, 1024, 1023, 28 * 24, 65 * 24, 5 * 24] {
            highs[row] += 1_000.;
            let bars = input(&times, &highs, &lows, &closes);
            state.update(bars, source, row);
            assert_eq!(
                state.outputs(),
                session_study(bars, source, kind),
                "repair {row} {kind:?}"
            );
        }
    }
}

#[test]
fn utc_day_levels_and_previous_day_wait_for_next_observed_period() {
    let times = [86_399, 86_400, 86_410, 3 * 86_400, 3 * 86_400 + 1];
    let highs = [12., 14., 17., 20., 21.];
    let lows = [8., 9., 7., 10., 9.];
    let closes = [10., 11., 15., 18., 19.];
    let bars = input(&times, &highs, &lows, &closes);
    assert_eq!(
        session_study(bars, SessionSource::Utc, SessionStudy::SessionLevels),
        [
            point(12., 8., None),
            point(14., 9., None),
            point(17., 7., None),
            point(20., 10., None),
            point(21., 9., None),
        ]
    );
    assert_eq!(
        session_study(
            bars,
            SessionSource::Utc,
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        ),
        [
            SessionStudyPoint::default(),
            point(12., 8., Some(10.)),
            point(12., 8., Some(10.)),
            point(17., 7., Some(15.)),
            point(17., 7., Some(15.)),
        ]
    );
}

#[test]
fn monday_weeks_and_civil_months_include_negative_epoch_dates() {
    // 1969-12-28 Sun, Dec 29 Mon, Dec 31 Wed, Jan 1 Thu, Jan 5 Mon.
    let days = [-4, -3, -1, 0, 4];
    let times = days.map(|day| day * 86_400);
    let highs = [10., 11., 12., 13., 14.];
    let lows = [1., 2., 3., 4., 5.];
    let closes = [6., 7., 8., 9., 10.];
    let bars = input(&times, &highs, &lows, &closes);
    let none = SessionStudyPoint::default();
    assert_eq!(
        session_study(
            bars,
            SessionSource::Utc,
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Week),
        ),
        [
            none,
            point(10., 1., Some(6.)),
            point(10., 1., Some(6.)),
            point(10., 1., Some(6.)),
            point(13., 2., Some(9.)),
        ]
    );
    assert_eq!(
        session_study(
            bars,
            SessionSource::Utc,
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Month),
        ),
        [
            none,
            none,
            none,
            point(12., 1., Some(8.)),
            point(12., 1., Some(8.)),
        ]
    );
}

#[test]
fn host_adjacent_spans_merge_and_gaps_emit_no_levels() {
    let spans = [
        SessionSpan {
            start: 86_390,
            end: 86_400,
            session_id: 7,
        },
        SessionSpan {
            start: 86_400,
            end: 86_420,
            session_id: 7,
        },
        SessionSpan {
            start: 86_430,
            end: 86_450,
            session_id: 8,
        },
    ];
    let times = [86_395, 86_405, 86_425, 86_435, 86_445];
    let highs = [11., 13., 100., 17., 18.];
    let lows = [5., 4., 0., 7., 6.];
    let closes = [8., 9., 90., 12., 14.];
    let bars = input(&times, &highs, &lows, &closes);
    assert_eq!(
        session_study(
            bars,
            SessionSource::Host(&spans),
            SessionStudy::SessionLevels
        ),
        [
            point(11., 5., None),
            point(13., 4., None),
            SessionStudyPoint::default(),
            point(17., 7., None),
            point(18., 6., None),
        ]
    );
    // Trading date for both pieces of session 7 is end-1 = day 1, not the
    // civil date of its first bar (day 0). Session 8 remains day 1.
    assert_eq!(
        session_study(
            bars,
            SessionSource::Host(&spans),
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        ),
        [SessionStudyPoint::default(); 5]
    );
    let later = [
        spans[0],
        spans[1],
        SessionSpan {
            start: 2 * 86_400,
            end: 2 * 86_400 + 20,
            session_id: 9,
        },
    ];
    let times = [86_395, 86_405, 2 * 86_400 + 2];
    let highs = [11., 13., 17.];
    let lows = [5., 4., 7.];
    let closes = [8., 9., 12.];
    assert_eq!(
        session_study(
            input(&times, &highs, &lows, &closes),
            SessionSource::Host(&later),
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        ),
        [
            SessionStudyPoint::default(),
            SessionStudyPoint::default(),
            point(13., 4., Some(9.))
        ]
    );
}

#[test]
fn opening_range_uses_elapsed_seconds_and_freezes_within_session() {
    let spans = [
        SessionSpan {
            start: 100,
            end: 105,
            session_id: 4,
        },
        SessionSpan {
            start: 105,
            end: 120,
            session_id: 4,
        },
        SessionSpan {
            start: 130,
            end: 150,
            session_id: 4,
        },
    ];
    let times = [100, 104, 105, 109, 110, 115, 125, 130, 140];
    let highs = [10., 11., 12., 13., 99., 98., 200., 14., 15.];
    let lows = [5., 4., 3., 2., 0., -1., -2., 6., 5.];
    let closes = [8., 8., 8., 8., 8., 8., 8., 8., 8.];
    assert_eq!(
        session_study(
            input(&times, &highs, &lows, &closes),
            SessionSource::Host(&spans),
            SessionStudy::OpeningRange {
                duration_seconds: 10
            },
        ),
        [
            point(10., 5., None),
            point(11., 4., None),
            point(12., 3., None),
            point(13., 2., None),
            point(13., 2., None),
            point(13., 2., None),
            SessionStudyPoint::default(),
            point(14., 6., None),
            point(14., 6., None),
        ]
    );
}

#[test]
fn host_trading_day_controls_week_and_month_even_when_start_crosses_boundary() {
    // Sunday Dec 28 -> Monday Dec 29, and Dec 31 -> Jan 1.
    let spans = [
        SessionSpan {
            start: -4 * 86_400,
            end: -2 * 86_400,
            session_id: 1,
        },
        SessionSpan {
            start: -86_400,
            end: 86_400,
            session_id: 2,
        },
        SessionSpan {
            start: 4 * 86_400,
            end: 5 * 86_400,
            session_id: 3,
        },
    ];
    let times = [-4 * 86_400, -86_400, 4 * 86_400];
    let highs = [10., 20., 30.];
    let lows = [5., 7., 9.];
    let closes = [8., 18., 28.];
    let bars = input(&times, &highs, &lows, &closes);
    // Session 1 ends on Dec 29 (Monday), session 2 ends on Jan 1
    // (same week), session 3 ends on Jan 5 (next Monday week).
    assert_eq!(
        session_study(
            bars,
            SessionSource::Host(&spans),
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Week)
        ),
        [
            SessionStudyPoint::default(),
            SessionStudyPoint::default(),
            point(20., 5., Some(18.))
        ]
    );
    assert_eq!(
        session_study(
            bars,
            SessionSource::Host(&spans),
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Month)
        ),
        [
            SessionStudyPoint::default(),
            point(10., 5., Some(8.)),
            point(10., 5., Some(8.))
        ]
    );
}

#[test]
fn blank_boundary_row_does_not_replace_completed_period() {
    let times = [0, 86_400, 86_401];
    let highs = [10., f64::NAN, 20.];
    let lows = [5., f64::NAN, 8.];
    let closes = [7., f64::NAN, 15.];
    assert_eq!(
        session_study(
            input(&times, &highs, &lows, &closes),
            SessionSource::Utc,
            SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day)
        ),
        [
            SessionStudyPoint::default(),
            SessionStudyPoint::default(),
            point(10., 5., Some(7.))
        ]
    );
}

#[test]
fn entirely_blank_period_carries_last_observed_previous_levels() {
    const DAY: i64 = 86_400;
    for (period, days) in [
        (PreviousPeriod::Day, [0, 1, 2]),
        (PreviousPeriod::Week, [4, 11, 18]),
        (PreviousPeriod::Month, [0, 31, 59]),
    ] {
        let times = [
            days[0] * DAY,
            days[1] * DAY,
            days[1] * DAY + 60,
            days[1] * DAY + 120,
            days[2] * DAY,
            days[2] * DAY + 60,
        ];
        let highs = [10., f64::NAN, f64::NAN, f64::NAN, 20., 21.];
        let lows = [5., f64::NAN, f64::NAN, f64::NAN, 8., 7.];
        let closes = [7., f64::NAN, f64::NAN, f64::NAN, 15., 16.];
        let bars = input(&times, &highs, &lows, &closes);
        let kind = SessionStudy::PreviousPeriodLevels(period);
        let expected = [
            SessionStudyPoint::default(),
            SessionStudyPoint::default(),
            SessionStudyPoint::default(),
            SessionStudyPoint::default(),
            point(10., 5., Some(7.)),
            point(10., 5., Some(7.)),
        ];
        let valid = [0, 4, 5];
        let missing_times = valid.map(|row| times[row]);
        let missing_highs = valid.map(|row| highs[row]);
        let missing_lows = valid.map(|row| lows[row]);
        let missing_closes = valid.map(|row| closes[row]);
        let missing = input(
            &missing_times,
            &missing_highs,
            &missing_lows,
            &missing_closes,
        );
        let dense = session_study(bars, SessionSource::Utc, kind);
        let missing_dense = session_study(missing, SessionSource::Utc, kind);
        assert_eq!(dense, expected, "{period:?}");
        assert_eq!(
            valid.map(|row| dense[row]),
            missing_dense.as_slice(),
            "dense blank vs missing {period:?}"
        );
        let mut state = SessionStudyState::new(kind);
        let mut missing_state = SessionStudyState::new(kind);
        for end in 1..=times.len() {
            state.update(
                input(&times[..end], &highs[..end], &lows[..end], &closes[..end]),
                SessionSource::Utc,
                end - 1,
            );
            let missing_end = valid.iter().filter(|&&row| row < end).count();
            missing_state.update(
                input(
                    &missing_times[..missing_end],
                    &missing_highs[..missing_end],
                    &missing_lows[..missing_end],
                    &missing_closes[..missing_end],
                ),
                SessionSource::Utc,
                missing_end.saturating_sub(1),
            );
            assert_eq!(state.outputs(), &expected[..end], "append {end} {period:?}");
            let observed: Vec<_> = valid
                .iter()
                .copied()
                .filter(|&row| row < end)
                .map(|row| state.outputs()[row])
                .collect();
            assert_eq!(
                observed,
                missing_state.outputs(),
                "incremental blank vs missing {period:?} at {end}"
            );
        }
    }
}

#[test]
fn host_blank_session_and_historical_repair_preserve_period_semantics() {
    const DAY: i64 = 86_400;
    let spans = [0, 1, 2].map(|day| SessionSpan {
        start: day * DAY,
        end: (day + 1) * DAY,
        session_id: day as u64,
    });
    let times = [0, DAY, DAY + 1, DAY + 2, 2 * DAY];
    let mut highs = [10., f64::NAN, f64::NAN, f64::NAN, 20.];
    let mut lows = [5., f64::NAN, f64::NAN, f64::NAN, 8.];
    let mut closes = [7., f64::NAN, f64::NAN, f64::NAN, 15.];
    let source = SessionSource::Host(&spans);
    let missing_spans = [spans[0], spans[2]];
    let missing_times = [times[0], times[4]];
    let missing_highs = [highs[0], highs[4]];
    let missing_lows = [lows[0], lows[4]];
    let missing_closes = [closes[0], closes[4]];
    let missing = input(
        &missing_times,
        &missing_highs,
        &missing_lows,
        &missing_closes,
    );
    let missing_source = SessionSource::Host(&missing_spans);
    for kind in [
        SessionStudy::SessionLevels,
        SessionStudy::OpeningRange {
            duration_seconds: 60,
        },
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
    ] {
        let mut state = SessionStudyState::new(kind);
        let expected = match kind {
            SessionStudy::PreviousPeriodLevels(_) => [
                SessionStudyPoint::default(),
                SessionStudyPoint::default(),
                SessionStudyPoint::default(),
                SessionStudyPoint::default(),
                point(10., 5., Some(7.)),
            ],
            _ => [
                point(10., 5., None),
                SessionStudyPoint::default(),
                SessionStudyPoint::default(),
                SessionStudyPoint::default(),
                point(20., 8., None),
            ],
        };
        let bars = input(&times, &highs, &lows, &closes);
        let dense = session_study(bars, source, kind);
        assert_eq!(dense, expected, "{kind:?}");
        assert_eq!(
            [dense[0], dense[4]],
            session_study(missing, missing_source, kind).as_slice(),
            "dense blank vs missing host session {kind:?}"
        );
        let mut missing_state = SessionStudyState::new(kind);
        for end in 1..=times.len() {
            state.update(
                input(&times[..end], &highs[..end], &lows[..end], &closes[..end]),
                source,
                end - 1,
            );
            let missing_end = usize::from(end > 0) + usize::from(end > 4);
            missing_state.update(
                input(
                    &missing_times[..missing_end],
                    &missing_highs[..missing_end],
                    &missing_lows[..missing_end],
                    &missing_closes[..missing_end],
                ),
                missing_source,
                missing_end.saturating_sub(1),
            );
        }
        assert_eq!(state.outputs(), expected, "initial {kind:?}");
        assert_eq!(
            [state.outputs()[0], state.outputs()[4]],
            missing_state.outputs(),
            "incremental blank vs missing host session {kind:?}"
        );

        highs[1] = 12.;
        lows[1] = 4.;
        closes[1] = 9.;
        let bars = input(&times, &highs, &lows, &closes);
        state.update(bars, source, 1);
        assert_eq!(
            state.outputs(),
            session_study(bars, source, kind),
            "repair {kind:?}"
        );
        if matches!(kind, SessionStudy::PreviousPeriodLevels(_)) {
            assert_eq!(state.outputs()[4], point(12., 4., Some(9.)));
        }
        highs[1] = f64::NAN;
        lows[1] = f64::NAN;
        closes[1] = f64::NAN;
        let bars = input(&times, &highs, &lows, &closes);
        state.update(bars, source, 1);
        assert_eq!(state.outputs(), expected, "blank repair {kind:?}");
        assert_eq!(
            [state.outputs()[0], state.outputs()[4]],
            missing_state.outputs(),
            "repaired blank vs missing host session {kind:?}"
        );
    }
}

#[test]
fn whitespace_prefixes_and_tip_corrections_recompute_without_lookahead() {
    let times = [0, 20, 40, 86_400, 86_420];
    let highs = [10., f64::NAN, 12., 15., 16.];
    let lows = [5., f64::NAN, 4., 8., 7.];
    let closes = [7., f64::NAN, 9., 11., 13.];
    for kind in [
        SessionStudy::SessionLevels,
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        SessionStudy::OpeningRange {
            duration_seconds: 30,
        },
    ] {
        let full = session_study(
            input(&times, &highs, &lows, &closes),
            SessionSource::Utc,
            kind,
        );
        for end in 0..=times.len() {
            assert_eq!(
                session_study(
                    input(&times[..end], &highs[..end], &lows[..end], &closes[..end]),
                    SessionSource::Utc,
                    kind,
                ),
                full[..end],
                "prefix {end} of {kind:?}"
            );
        }
        let mut corrected = highs;
        corrected[4] = 30.;
        let new = session_study(
            input(&times, &corrected, &lows, &closes),
            SessionSource::Utc,
            kind,
        );
        assert_eq!(new[..4], full[..4]);
        match kind {
            SessionStudy::SessionLevels => assert_eq!(new[4], point(30., 7., None)),
            SessionStudy::PreviousPeriodLevels(_) => assert_eq!(new[4], full[4]),
            SessionStudy::OpeningRange { .. } => assert_eq!(new[4], point(30., 7., None)),
        }
    }
}

#[test]
fn exchange_source_with_the_utc_calendar_equals_utc_and_replays_incrementally() {
    const DAY: i64 = 86_400;
    // Hourly bars from before the epoch (negative times floor) across several weeks and a month.
    let times: Vec<_> = (0..2_175).map(|row| (row as i64 - 300) * 3_600).collect();
    let mut highs: Vec<_> = (0..times.len())
        .map(|row| 100. + (row % 29) as f64)
        .collect();
    let mut lows: Vec<_> = (0..times.len())
        .map(|row| 70. - (row % 11) as f64)
        .collect();
    let closes: Vec<_> = (0..times.len()).map(|row| 90. + (row % 7) as f64).collect();
    let utc_day = |time: i64| time.div_euclid(DAY) * DAY;
    let identity = SessionSource::Exchange {
        trading_day_seconds: &utc_day,
    };
    // A +8h exchange whose trading day starts at 21:00 local (13:00 UTC).
    let night_day = |time: i64| (time + 11 * 3_600).div_euclid(DAY) * DAY;
    let night = SessionSource::Exchange {
        trading_day_seconds: &night_day,
    };
    for kind in [
        SessionStudy::SessionLevels,
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Day),
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Week),
        SessionStudy::PreviousPeriodLevels(PreviousPeriod::Month),
        SessionStudy::OpeningRange {
            duration_seconds: 2 * 3_600,
        },
    ] {
        let bars = input(&times, &highs, &lows, &closes);
        assert_eq!(
            session_study(bars, identity, kind),
            session_study(bars, SessionSource::Utc, kind),
            "{kind:?}"
        );
        let mut state = SessionStudyState::new(kind);
        for (previous, end) in [(0, 1), (1, 1023), (1023, 1025), (1025, times.len())] {
            let bars = input(&times[..end], &highs[..end], &lows[..end], &closes[..end]);
            state.update(bars, night, previous);
            assert_eq!(
                state.outputs(),
                session_study(bars, night, kind),
                "{kind:?}"
            );
        }
        for row in [times.len() - 1, 1500, 1024, 40] {
            highs[row] += 250.;
            lows[row] -= 250.;
            let bars = input(&times, &highs, &lows, &closes);
            state.update(bars, night, row);
            assert_eq!(
                state.outputs(),
                session_study(bars, night, kind),
                "{kind:?}"
            );
        }
        // Switching from the exchange source to UTC replays every row.
        let bars = input(&times, &highs, &lows, &closes);
        state.update(bars, SessionSource::Utc, times.len());
        assert_eq!(
            state.outputs(),
            session_study(bars, SessionSource::Utc, kind)
        );
        assert_eq!(state.last_work_rows(), times.len());
    }
    // The night-session opening range is anchored at the trading day's first bar (21:00 local,
    // 13:00 UTC) and the bar at UTC midnight stays in the same trading day: the two-hour range
    // covers 13:00 and 14:00 only.
    let day_start = 13 * 3_600;
    let window = [day_start, day_start + 3_600, day_start + 2 * 3_600, DAY];
    let range = session_study(
        input(
            &window,
            &[10., 12., 30., 40.],
            &[5., 4., 1., 0.],
            &[7., 7., 7., 7.],
        ),
        night,
        SessionStudy::OpeningRange {
            duration_seconds: 2 * 3_600,
        },
    );
    assert_eq!(
        range,
        [
            point(10., 5., None),
            point(12., 4., None),
            point(12., 4., None),
            point(12., 4., None),
        ]
    );
}

/// 15-minute bars from 01:30 UTC (09:30 in a +8h zone) for `days` days: an A-share-style
/// market whose first bar is later than the UTC midnight or exchange session start.
fn late_open_bars(days: i64, per_day: i64) -> Vec<i64> {
    (0..days)
        .flat_map(|day| (0..per_day).map(move |bar| day * 86_400 + 5_400 + bar * 900))
        .collect()
}

#[test]
fn opening_range_starts_at_the_first_bar_when_the_market_opens_after_the_session_start() {
    const DAY: i64 = 86_400;
    let times = [5_400, 6_300, 7_200, 8_100, DAY + 5_400, DAY + 7_200];
    let highs = [10., 12., 30., 40., 20., 25.];
    let lows = [5., 4., 1., 0., 15., 14.];
    let closes = [7.; 6];
    let bars = input(&times, &highs, &lows, &closes);
    let kind = SessionStudy::OpeningRange {
        duration_seconds: 1_800,
    };
    // 09:30-10:00 local: the 09:30 and 09:45 bars form the range; 10:00 and later keep it.
    let expected = [
        point(10., 5., None),
        point(12., 4., None),
        point(12., 4., None),
        point(12., 4., None),
        point(20., 15., None),
        point(20., 15., None),
    ];
    assert_eq!(session_study(bars, SessionSource::Utc, kind), expected);
    // The same market on a +8h exchange calendar whose session starts at local midnight.
    let local_day = |time: i64| (time + 8 * 3_600).div_euclid(DAY) * DAY;
    let exchange = SessionSource::Exchange {
        trading_day_seconds: &local_day,
    };
    assert_eq!(session_study(bars, exchange, kind), expected);
}

#[test]
fn opening_range_anchors_at_the_first_valid_row_after_a_whitespace_opening_bar() {
    let times = [5_400, 6_300, 7_200, 8_100];
    let highs = [f64::NAN, 12., 30., 40.];
    let lows = [f64::NAN, 4., 1., 0.];
    let closes = [f64::NAN, 7., 7., 7.];
    let bars = input(&times, &highs, &lows, &closes);
    let kind = SessionStudy::OpeningRange {
        duration_seconds: 1_800,
    };
    // The missing 09:30 bar does not anchor: the range runs 09:45-10:15.
    let expected = [
        SessionStudyPoint::default(),
        point(12., 4., None),
        point(30., 1., None),
        point(30., 1., None),
    ];
    assert_eq!(session_study(bars, SessionSource::Utc, kind), expected);
    // A host span that opens on the whitespace row anchors at the first valid row as well.
    let spans = [SessionSpan {
        start: 5_400,
        end: 9_000,
        session_id: 1,
    }];
    assert_eq!(
        session_study(bars, SessionSource::Host(&spans), kind),
        expected
    );
}

#[test]
fn opening_range_reanchors_through_tip_replacement_and_checkpoint_replay() {
    // 32 bars per session, so row 1024 (a checkpoint) is the first row of session 32.
    let times = late_open_bars(40, 32);
    let mut highs: Vec<_> = (0..times.len())
        .map(|row| 100. + (row % 32) as f64)
        .collect();
    let mut lows: Vec<_> = (0..times.len())
        .map(|row| 50. - (row % 32) as f64)
        .collect();
    let mut closes = vec![75.; times.len()];
    let kind = SessionStudy::OpeningRange {
        duration_seconds: 1_800,
    };
    let blank = |highs: &mut [f64], lows: &mut [f64], closes: &mut [f64], row: usize| {
        highs[row] = f64::NAN;
        lows[row] = f64::NAN;
        closes[row] = f64::NAN;
    };
    let fill = |highs: &mut [f64], lows: &mut [f64], closes: &mut [f64], row: usize| {
        highs[row] = 500. + row as f64;
        lows[row] = 1.;
        closes[row] = 250.;
    };
    let mut state = SessionStudyState::new(kind);
    // The tip is the whitespace first row of a session, then becomes a valid bar, then blank again.
    let tip = 3 * 32;
    blank(&mut highs, &mut lows, &mut closes, tip);
    let prefix = |highs: &[f64], lows: &[f64], closes: &[f64], end: usize| {
        (
            times[..end].to_vec(),
            highs[..end].to_vec(),
            lows[..end].to_vec(),
            closes[..end].to_vec(),
        )
    };
    for step in 0..3 {
        match step {
            1 => fill(&mut highs, &mut lows, &mut closes, tip),
            2 => blank(&mut highs, &mut lows, &mut closes, tip),
            _ => {}
        }
        let (t, h, l, c) = prefix(&highs, &lows, &closes, tip + 1);
        let bars = input(&t, &h, &l, &c);
        state.update(bars, SessionSource::Utc, if step == 0 { 0 } else { tip });
        assert_eq!(
            state.outputs(),
            session_study(bars, SessionSource::Utc, kind),
            "tip step {step}"
        );
        let expected = if step == 1 {
            point(500. + tip as f64, 1., None)
        } else {
            SessionStudyPoint::default()
        };
        assert_eq!(state.outputs()[tip], expected, "tip step {step}");
    }
    // Append the next bar: with the whitespace first row, it anchors the range itself.
    let (t, h, l, c) = prefix(&highs, &lows, &closes, tip + 3);
    let bars = input(&t, &h, &l, &c);
    state.update(bars, SessionSource::Utc, tip + 1);
    assert_eq!(
        state.outputs(),
        session_study(bars, SessionSource::Utc, kind)
    );
    assert_eq!(
        state.outputs()[tip + 1..],
        [
            point(highs[tip + 1], lows[tip + 1], None),
            point(highs[tip + 2], lows[tip + 2], None)
        ]
    );
    // Historical corrections of a session's first row across the 1024-row checkpoint.
    let bars = input(&times, &highs, &lows, &closes);
    state.update(bars, SessionSource::Utc, tip + 3);
    assert_eq!(
        state.outputs(),
        session_study(bars, SessionSource::Utc, kind)
    );
    for (row, valid) in [(1024, false), (1024, true), (1023 - 31, false), (tip, true)] {
        if valid {
            fill(&mut highs, &mut lows, &mut closes, row);
        } else {
            blank(&mut highs, &mut lows, &mut closes, row);
        }
        let bars = input(&times, &highs, &lows, &closes);
        state.update(bars, SessionSource::Utc, row);
        let full = session_study(bars, SessionSource::Utc, kind);
        assert_eq!(state.outputs(), full, "repair {row} valid {valid}");
        // 09:30 (or 09:45 after a blank opening bar) through 10:00 or 10:15 local.
        let first = if valid { row } else { row + 1 };
        let high = highs[first].max(highs[first + 1]);
        let low = lows[first].min(lows[first + 1]);
        assert_eq!(full[first + 2], point(high, low, None), "repair {row}");
        assert_eq!(full[row + 31], point(high, low, None), "repair {row}");
    }

    // 30 bars per session: session 34 opens at row 1020 and straddles the 1024-row checkpoint,
    // whose runtime must carry the anchor (row 1021, after a whitespace opening bar).
    let times = late_open_bars(40, 30);
    let mut highs: Vec<_> = (0..times.len())
        .map(|row| 100. + (row % 30) as f64)
        .collect();
    let mut lows = vec![10.; times.len()];
    let mut closes = vec![50.; times.len()];
    blank(&mut highs, &mut lows, &mut closes, 1020);
    let kind = SessionStudy::OpeningRange {
        duration_seconds: 2 * 3_600,
    };
    let mut state = SessionStudyState::new(kind);
    state.update(input(&times, &highs, &lows, &closes), SessionSource::Utc, 0);
    // 09:45-11:45 local covers rows 1021-1028; row 1029 (11:45) is outside.
    for (row, high, expected) in [(1028, 900., 900.), (1029, 2_000., 900.), (1024, 950., 950.)] {
        highs[row] = high;
        let bars = input(&times, &highs, &lows, &closes);
        state.update(bars, SessionSource::Utc, row);
        assert_eq!(
            state.outputs(),
            session_study(bars, SessionSource::Utc, kind),
            "straddling repair {row}"
        );
        assert_eq!(
            state.outputs()[1049],
            point(expected, 10., None),
            "repair {row}"
        );
    }
}
