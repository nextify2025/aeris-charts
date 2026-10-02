//! aeris_charts_core — platform-free chart model.
//!
//! Independent chart-model implementation informed by public financial-chart APIs and observed
//! behavior. All model math is `f64`; conversion to backend coordinate formats happens at render
//! encoding.

pub mod format;
pub mod helpers;
pub mod model;
pub mod options;
pub mod scale;
pub mod style;
pub mod time_zone;

/// Media-space (CSS px) coordinate. Bitmap conversion happens at encode time only.
pub type Coordinate = f64;

/// Integer index into the merged time-scale point list. May be negative in logical space
/// (positions left of the first bar).
pub type TimePointIndex = i64;
