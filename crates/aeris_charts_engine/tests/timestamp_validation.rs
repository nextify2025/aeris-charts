use aeris_charts_core::model::data_validation::{
    MAX_TIMESTAMP, MIN_TIMESTAMP, TimestampErrorCategory, ValidationError,
};
use aeris_charts_engine::ChartEngine;

#[test]
fn direct_series_batches_and_updates_preserve_state_on_invalid_timestamps() {
    let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
    let values = [10.0, 20.0];
    chart
        .set_series_data(
            0,
            &[MIN_TIMESTAMP as f64, MAX_TIMESTAMP as f64],
            &values,
            &values,
            &values,
            &values,
        )
        .unwrap();
    let before = chart.series_data(0);

    for (invalid, category) in [
        (f64::NAN, TimestampErrorCategory::NonFinite),
        (1.5, TimestampErrorCategory::Fractional),
        (
            MAX_TIMESTAMP as f64 + 1.0,
            TimestampErrorCategory::OutOfRange,
        ),
        (1_725_000_000_000.0, TimestampErrorCategory::OutOfRange),
    ] {
        let error = chart
            .set_series_data(0, &[100.0, invalid], &values, &values, &values, &values)
            .unwrap_err();
        assert!(matches!(
            error,
            ValidationError::InvalidTimestamp {
                index: 1,
                error: aeris_charts_core::model::data_validation::TimestampError {
                    category: actual,
                    ..
                }
            } if actual == category
        ));
        assert_eq!(chart.series_data(0), before);
        assert!(!chart.update_series_bar(0, invalid, [30.0; 4]));
        assert_eq!(chart.series_data(0), before);
    }
}
