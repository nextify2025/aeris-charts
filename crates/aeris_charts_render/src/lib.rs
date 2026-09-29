//! aeris_charts_render — backend-agnostic draw-list IR and rendering math.
//!
//! The headless engine emits [`draw_list::DrawList`]s; GPUI, WebGPU, Canvas2D, and native
//! executors consume the same ordered primitive stream.

pub mod bar_width;
pub mod bars;
pub mod candles;
pub mod canvas2d;
pub mod color;
pub mod crosshair_icon;
pub mod draw_list;
pub mod histogram;
pub mod line;
pub mod shape;
