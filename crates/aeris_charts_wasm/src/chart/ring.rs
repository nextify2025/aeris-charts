//! `SharedArrayBuffer` ring data source: the browser half of [`crate::ring_source`].
//!
//! A series can be bound to a caller-owned ring that a Web Worker fills. The engine then drains it
//! once per frame instead of the host calling in per tick, which is what makes the cost per frame
//! independent of the tick rate: 5 ticks/sec and 50,000 ticks/sec both cost one atomic load plus
//! one bulk copy of whatever accumulated.
//!
//! **Why a bulk copy rather than true in-place reads.** wasm cannot address a `SharedArrayBuffer`
//! directly — every read of it crosses the JS boundary. Reading five `f64`s per row through a
//! `DataView` would be five JS calls per row, which is worse than the object allocation this API
//! exists to remove. Instead each drain copies the contiguous run(s) of new row bytes into a
//! reused wasm-side slab in one `TypedArray.set` per run, then parses rows in Rust with no further
//! boundary crossings. Nothing is materialized as a JS value, and no allocation happens per row or
//! per frame — which is the property the consumer actually needs.

use aeris_charts_core::model::data_layer::SeriesId;
use wasm_bindgen::JsValue;

use crate::ring_source::RingLayout;

/// Layout as it arrives from the façade (the public `ring_source_layout`, snake_case).
#[derive(serde::Deserialize)]
pub(super) struct RingLayoutInput {
    data_offset: f64,
    row_stride: f64,
    capacity: f64,
    time_offset: f64,
    open_offset: f64,
    high_offset: f64,
    low_offset: f64,
    close_offset: f64,
    write_cursor_offset: f64,
    /// Optional Int32 sequence word within every row. When present, the producer writes
    /// `!next_cursor`, then the row bytes, then `next_cursor`, and finally publishes the global
    /// cursor. This per-slot seqlock lets the consumer prove a wrap did not overwrite a row while
    /// it was being copied.
    #[serde(default)]
    sequence_offset: Option<f64>,
}

impl RingLayoutInput {
    /// Narrow to the engine's layout. Non-finite or negative fields collapse to 0, which the
    /// layout validation then rejects with a specific message.
    fn to_layout(&self) -> (RingLayout, Option<usize>) {
        let at = |v: f64| {
            if v.is_finite() && v >= 0.0 {
                v as usize
            } else {
                0
            }
        };
        (
            RingLayout {
                data_offset: at(self.data_offset),
                row_stride: at(self.row_stride),
                capacity: at(self.capacity),
                time_offset: at(self.time_offset),
                open_offset: at(self.open_offset),
                high_offset: at(self.high_offset),
                low_offset: at(self.low_offset),
                close_offset: at(self.close_offset),
                write_cursor_offset: at(self.write_cursor_offset),
            },
            self.sequence_offset.map(at),
        )
    }
}

pub(super) struct BoundRing {
    pub(super) series_id: SeriesId,
    /// Byte view over the whole shared buffer; the source of the bulk slab copies.
    bytes: js_sys::Uint8Array,
    /// `Int32` view over the same buffer, for the atomic cursor load.
    cursor_view: js_sys::Int32Array,
    layout: RingLayout,
    /// Optional per-row seqlock word, relative to the row start.
    sequence_offset: Option<usize>,
    /// Rows consumed so far, in the producer's monotonic row count.
    consumed: i32,
    /// Reused row-bytes staging area, sized once to `capacity * row_stride` so a drain — even a
    /// full-ring overrun drain — never allocates.
    slab: Vec<u8>,
}

/// Outcome of draining one ring for one frame.
pub(super) struct DrainOutcome {
    pub(super) rows: u32,
    /// True when a published candidate window was inspected, including unstable retries or a
    /// stable window whose rows were all rejected by sanitization.
    pub(super) had_work: bool,
    /// Rows the producer overwrote before this drain reached them.
    pub(super) lost_rows: u32,
    /// Rows read from the ring but dropped as invalid (bad timestamp or non-finite values).
    pub(super) dropped_rows: u32,
}

impl BoundRing {
    /// Bind a ring, or report why the layout is unusable. `bytes` and `cursor_view` must be views
    /// over the same buffer — the façade constructs both from the one it was handed.
    pub(super) fn new(
        series_id: SeriesId,
        bytes: js_sys::Uint8Array,
        cursor_view: js_sys::Int32Array,
        layout: &RingLayoutInput,
    ) -> Result<Self, String> {
        let (layout, sequence_offset) = layout.to_layout();
        layout
            .validate(bytes.length() as usize)
            .map_err(|e| e.to_string())?;
        if let Some(offset) = sequence_offset {
            if offset.saturating_add(4) > layout.row_stride {
                return Err(format!(
                    "sequence_offset {offset} + 4 bytes exceeds row_stride {}",
                    layout.row_stride
                ));
            }
            if !(layout.data_offset + offset).is_multiple_of(4)
                || !layout.row_stride.is_multiple_of(4)
            {
                return Err(
                    "sequence_offset must address a 4-byte-aligned Int32 in every row".into(),
                );
            }
            let sequence_end = offset + 4;
            for (name, channel) in [
                ("time", layout.time_offset),
                ("open", layout.open_offset),
                ("high", layout.high_offset),
                ("low", layout.low_offset),
                ("close", layout.close_offset),
            ] {
                let channel_end = channel + std::mem::size_of::<f64>();
                if offset < channel_end && channel < sequence_end {
                    return Err(format!(
                        "sequence_offset {offset} overlaps {name} channel bytes {channel}..{channel_end}"
                    ));
                }
            }
        }
        // Start from the producer's current cursor: binding mid-stream picks up new rows rather
        // than replaying whatever happens to be sitting in the ring.
        let consumed = load_cursor(&cursor_view, layout.cursor_index()).unwrap_or(0);
        Ok(Self {
            series_id,
            bytes,
            cursor_view,
            layout,
            sequence_offset,
            consumed,
            slab: vec![0u8; layout.capacity * layout.row_stride],
        })
    }

    /// Read whatever the producer has published since the last drain and apply it to the series.
    ///
    /// Rows are applied in ring order through the same engine entry point `update()` uses, so each
    /// row appends or replaces-last and a non-finite row is dropped. The batch sort/dedupe that
    /// `update_typed` applies is deliberately *not* run here: a ring is a stream whose producer
    /// already writes in order, and re-sorting a frame's window would need a per-frame allocation.
    pub(super) fn drain(&mut self, engine: &mut aeris_charts_engine::ChartEngine) -> DrainOutcome {
        // A producer can wrap while this function copies. With sequence words, retry until every
        // selected slot still contains the exact logical row expected both before and after the
        // bulk copy. Three attempts keep one pathological producer from monopolizing a frame; a
        // failed attempt consumes nothing and the next frame retries from the same cursor.
        const MAX_COPY_ATTEMPTS: usize = 3;
        for _ in 0..MAX_COPY_ATTEMPTS {
            let Some(cursor) = load_cursor(&self.cursor_view, self.layout.cursor_index()) else {
                return DrainOutcome {
                    rows: 0,
                    had_work: false,
                    lost_rows: 0,
                    dropped_rows: 0,
                };
            };
            let plan = self.layout.plan_drain(self.consumed, cursor);
            let take = plan.rows();
            if take == 0 {
                self.consumed = plan.consumed_to;
                return DrainOutcome {
                    rows: 0,
                    had_work: false,
                    lost_rows: 0,
                    dropped_rows: 0,
                };
            }
            let start_count = cursor.wrapping_sub(take as i32);
            if !self.sequences_match(&plan, start_count) {
                continue;
            }

            // Copy every segment into one contiguous slab before applying any row. This makes a
            // failed stability check fully retryable and still costs at most two bulk copies.
            let mut slab_offset = 0usize;
            for segment in plan.segments() {
                let bytes = segment.count * self.layout.row_stride;
                let start = self.layout.slot_offset(segment.slot);
                self.bytes
                    .subarray(start as u32, (start + bytes) as u32)
                    .copy_to(&mut self.slab[slab_offset..slab_offset + bytes]);
                slab_offset += bytes;
            }

            let stable = if self.sequence_offset.is_some() {
                self.sequences_match(&plan, start_count)
            } else {
                // Legacy layouts remain source-compatible. A second cursor read detects a fully
                // published wrap into the copied slots; per-row sequence words are required for
                // the stronger guarantee against a writer that is currently mid-row.
                load_cursor(&self.cursor_view, self.layout.cursor_index()).is_some_and(|after| {
                    let advanced = after.wrapping_sub(cursor);
                    advanced >= 0 && (advanced as usize) <= self.layout.capacity - take
                })
            };
            if !stable {
                continue;
            }

            let stride = self.layout.row_stride;
            let [
                time_offset,
                open_offset,
                high_offset,
                low_offset,
                close_offset,
            ] = [
                self.layout.time_offset,
                self.layout.open_offset,
                self.layout.high_offset,
                self.layout.low_offset,
                self.layout.close_offset,
            ];
            let slab = &self.slab;
            let rows = (0..take).map(|row| {
                let base = row * stride;
                let f = |offset: usize| read_f64(slab, base + offset);
                (
                    f(time_offset),
                    [
                        f(open_offset),
                        f(high_offset),
                        f(low_offset),
                        f(close_offset),
                    ],
                )
            });
            let rows = engine.update_series_bars(self.series_id, rows) as u32;
            self.consumed = plan.consumed_to;
            return DrainOutcome {
                rows,
                had_work: true,
                lost_rows: plan.lost_rows as u32,
                dropped_rows: (take as u32).saturating_sub(rows),
            };
        }

        DrainOutcome {
            rows: 0,
            had_work: true,
            lost_rows: 0,
            dropped_rows: 0,
        }
    }

    /// Verify each selected slot's completed sequence equals the logical cursor value for that row.
    /// The producer writes the bitwise complement before touching channel bytes, so either a stale
    /// generation or a row currently being overwritten fails this check.
    fn sequences_match(&self, plan: &crate::ring_source::DrainPlan, start_count: i32) -> bool {
        let Some(sequence_offset) = self.sequence_offset else {
            return true;
        };
        let mut logical = start_count;
        for segment in plan.segments() {
            for slot in segment.slot..segment.slot + segment.count {
                logical = logical.wrapping_add(1);
                let byte_offset = self.layout.slot_offset(slot) + sequence_offset;
                if load_cursor(&self.cursor_view, (byte_offset / 4) as u32) != Some(logical) {
                    return false;
                }
            }
        }
        true
    }
}

/// `Atomics.load` on the producer's cursor. The atomic read is the synchronization edge: the
/// producer publishes the incremented count *after* writing the row, so every row below the value
/// read here is complete. `None` when the load throws (a detached or non-shared buffer), which
/// skips the frame rather than failing the render.
fn load_cursor(view: &js_sys::Int32Array, index: u32) -> Option<i32> {
    match js_sys::Atomics::load(view, index) {
        Ok(value) => Some(value),
        Err(error) => {
            report_once(&error);
            None
        }
    }
}

/// A cursor load that throws means the ring is broken (detached buffer, non-shared memory).
/// Warned once per occurrence class rather than per frame — a per-frame console write on a broken
/// ring would be its own performance problem.
fn report_once(error: &JsValue) {
    use std::cell::Cell;
    thread_local! {
        static REPORTED: Cell<bool> = const { Cell::new(false) };
    }
    REPORTED.with(|reported| {
        if !reported.replace(true) {
            web_sys::console::warn_1(
                &format!(
                    "aeris_charts: ring source cursor read failed, skipping drains ({error:?})"
                )
                .into(),
            );
        }
    });
}

/// Little-endian `f64` at `offset`. JS typed arrays and wasm linear memory are both
/// little-endian, so the slab bytes need no swapping. An out-of-range read cannot happen after
/// layout validation; it yields NaN rather than panicking if it ever does, and the engine's
/// per-point sanitizer then drops the row.
fn read_f64(slab: &[u8], offset: usize) -> f64 {
    slab.get(offset..offset + 8)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map_or(f64::NAN, f64::from_le_bytes)
}
