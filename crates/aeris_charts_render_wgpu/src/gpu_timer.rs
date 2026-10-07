//! Optional GPU-side frame timing through the WebGPU `timestamp-query` feature.
//!
//! The render pass writes a timestamp at its beginning and end; the pair is resolved into a
//! GPU buffer, copied to a mappable staging buffer, and read back asynchronously. Readback is
//! inherently a frame or two behind the submit that produced it, so [`GpuTimer::last_ms`] is
//! "the most recently *resolved* frame", not strictly the frame just encoded — the distinction
//! does not matter for a rolling p99 budget and it keeps the timer off the critical path.
//!
//! Exactly one readback is in flight at a time: while a map is pending, subsequent frames skip
//! their timestamp writes entirely, so the cost is bounded no matter the frame rate. A device
//! without `timestamp-query` never constructs a timer and the host reports `gpu_ms: null`.
//!
//! The `map_async` callback deliberately does nothing but publish a state transition through an
//! `AtomicU8` — the mapped bytes are drained by the *host* on its next [`GpuTimer::last_ms`]
//! call. That keeps every value the callback captures `Send + Sync` on both the native and the
//! wasm target, where `wgpu::Buffer` is neither.

use std::cell::Cell;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Bytes for two `u64` timestamps (the resolve destination and its staging copy).
const TIMESTAMP_PAIR_BYTES: u64 = 16;

/// No readback in flight; the next frame may sample.
const IDLE: u8 = 0;
/// `map_async` issued, waiting on the driver.
const PENDING: u8 = 1;
/// Mapped and ready for the host to drain.
const MAPPED: u8 = 2;
/// The map failed (e.g. the adapter cannot read timestamps); clean up the map and stop sampling.
const FAILED: u8 = 3;
/// Timestamp collection is unavailable for the rest of this timer's lifetime.
const DISABLED: u8 = 4;

pub struct GpuTimer {
    query_set: wgpu::QuerySet,
    /// `resolve_query_set` destination (GPU-only, `QUERY_RESOLVE | COPY_SRC`).
    resolve: wgpu::Buffer,
    /// Host-mappable copy of `resolve`, mapped asynchronously after each submit.
    readback: wgpu::Buffer,
    /// Nanoseconds per timestamp tick, from `Queue::get_timestamp_period`.
    period_ns: f32,
    /// Readback lifecycle; the only state the `map_async` callback touches.
    state: Arc<AtomicU8>,
    /// Last drained pass duration in ms; `None` until the first readback lands.
    last_ms: Cell<Option<f64>>,
}

impl GpuTimer {
    /// Create a timer, or `None` when the device lacks [`wgpu::Features::TIMESTAMP_QUERY`].
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame_timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame_timestamps_resolve"),
            size: TIMESTAMP_PAIR_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame_timestamps_readback"),
            size: TIMESTAMP_PAIR_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Some(Self {
            query_set,
            resolve,
            readback,
            period_ns: queue.get_timestamp_period(),
            state: Arc::new(AtomicU8::new(IDLE)),
            last_ms: Cell::new(None),
        })
    }

    /// Duration of the most recently resolved render pass, in ms (`None` before the first
    /// readback completes). Drains a landed readback as a side effect — this is the host-side
    /// half of the async handshake, so it must be the only place the buffer is read/unmapped.
    pub fn last_ms(&self) -> Option<f64> {
        match self.state.load(Ordering::Acquire) {
            MAPPED => {
                // A wrapped/decreasing pair (driver quirk, or a pass that never ran) is dropped
                // rather than reported as a bogus duration.
                let ticks = {
                    self.readback
                        .slice(..)
                        .get_mapped_range()
                        .ok()
                        .and_then(|view| {
                            let stamps: &[u64] = bytemuck::cast_slice(&view);
                            stamps.get(1)?.checked_sub(*stamps.first()?)
                        })
                };
                self.readback.unmap();
                if let Some(ticks) = ticks {
                    self.last_ms
                        .set(Some(ticks as f64 * f64::from(self.period_ns) / 1.0e6));
                }
                self.state.store(IDLE, Ordering::Release);
            }
            FAILED => {
                // WGPU retains the slice's map range even when the browser rejects mapAsync.
                // Unmapping is required before the buffer can be mapped again; disable sampling
                // as well because GPU timing is optional and repeated failures only add work.
                self.readback.unmap();
                self.last_ms.set(None);
                self.state.store(DISABLED, Ordering::Release);
            }
            _ => {}
        }
        self.last_ms.get()
    }

    /// Whether this frame should write timestamps (false while a readback is outstanding or a
    /// mapped sample is still waiting to be drained).
    fn should_sample(&self) -> bool {
        self.state.load(Ordering::Acquire) == IDLE
    }

    /// The pass descriptor's `timestamp_writes` for a sampling frame.
    fn pass_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Resolve this frame's query pair into the staging buffer. Called after the pass closes
    /// and before `encoder.finish()`.
    fn resolve_into_staging(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query_set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, TIMESTAMP_PAIR_BYTES);
    }

    /// Start the asynchronous readback for the frame just submitted.
    fn begin_readback(&self) {
        self.state.store(PENDING, Ordering::Release);
        let state = Arc::clone(&self.state);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let next = if result.is_ok() { MAPPED } else { FAILED };
                state.store(next, Ordering::Release);
            });
    }
}

/// The timestamp plumbing a single [`crate::render_frame`] call needs, resolved once up front so
/// the render path stays branch-light: the pass writes are `Some` only on a sampling frame.
pub(crate) struct FrameTimestamps<'t> {
    /// `Some` only when a timer exists *and* this frame is sampling.
    timer: Option<&'t GpuTimer>,
}

impl<'t> FrameTimestamps<'t> {
    pub(crate) fn new(timer: Option<&'t GpuTimer>) -> Self {
        Self {
            timer: timer.filter(|timer| timer.should_sample()),
        }
    }

    pub(crate) fn pass_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'t>> {
        self.timer.map(GpuTimer::pass_writes)
    }

    pub(crate) fn resolve_into_staging(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(timer) = self.timer {
            timer.resolve_into_staging(encoder);
        }
    }

    pub(crate) fn begin_readback(&self) {
        if let Some(timer) = self.timer {
            timer.begin_readback();
        }
    }
}
