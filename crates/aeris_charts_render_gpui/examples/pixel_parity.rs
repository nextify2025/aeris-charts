//! Pixel-parity harness: diff what **official GPUI actually rasterized** against Aeris's existing
//! `aeris_charts_native` rasterizer, fixture by fixture.
//!
//! GPUI owns presentation through `PlatformWindow::draw(&Scene)` and exposes no framebuffer readback,
//! so the pixels are obtained by capturing the harness window's client area from outside GPUI:
//! - Windows: through DWM (`tools/capture_window.ps1`, PrintWindow + PW_RENDERFULLCONTENT).
//! - Linux: from the X server (`support/x11_capture.rs`, `GetImage` on GPUI's own X11 window), so the
//!   harness runs headless under `xvfb-run` with a software Vulkan driver.
//!
//! Either way it is a read of the presented window — it does not patch or fork GPUI, inject a render
//! pass, share a device, or re-rasterize the chart on the CPU for the GPUI side, so it is outside
//! every §12 stop condition.
//!
//! Each fixture isolates one *cause*, so a residual is attributable rather than a single opaque
//! number: crisp rects (coordinates/colour/coverage — legitimately held to zero), tessellated
//! geometry (triangle-edge antialiasing), gradients (interpolation), text (glyph rasterization).
//!
//! ```text
//! cargo run -p aeris_charts_render_gpui --features gpui-backend --example pixel_parity
//! ```
//!
//! Headless Linux, one command (needs `xvfb` and `mesa-vulkan-drivers`). Run it from the
//! repository root; the dev profile is the one the gate limits were measured with:
//!
//! ```text
//! env -u WAYLAND_DISPLAY GPUI_X11_SCALE_FACTOR=1 \
//!   VK_ICD_FILENAMES="$(ls /usr/share/vulkan/icd.d/lvp_icd*.json | head -n 1)" \
//!   xvfb-run -a -s "-screen 0 2560x1600x24" \
//!   cargo run -p aeris_charts_render_gpui --features gpui-backend --example pixel_parity
//! ```
//!
//! Writes `<out>/<fixture>_gpui.png`, `_native.png`, and `_diff.png` per fixture, plus
//! `results.json` with exact diff counts, the residual by magnitude (`pixels_over`), the mean
//! absolute error against the reference shifted by one pixel (the alignment check), the GPU adapter
//! GPUI used, and SHA-256 hashes for the images. A fixture's three images are deleted before its
//! capture, so none can come from an earlier run, and a row whose capture failed carries no GPUI or
//! diff hash. The process exits non-zero when a gate fails, when a capture does not come back at the
//! fixture size, when `results.json` cannot be written, or when the run does not finish within
//! [`DEADLINE`] (a missing X display stops GPUI from painting, so nothing else would end the run);
//! `results.json` and the images are written first. Environment knobs:
//! - `AERIS_CHARTS_PARITY_OUT`  — output directory (default `target/pixel_parity`).
//! - `AERIS_CHARTS_PARITY_TOL`  — per-channel tolerance for the `differing` column (default 0 —
//!   exact). The gates read `pixels_over` and do not depend on it.
//! - `GPUI_X11_SCALE_FACTOR`    — Linux/X11 only: the display scale GPUI uses (for example `1.5`),
//!   so fractional device-pixel ratios can be exercised without a scaled monitor.
//!
//! ## What the gates mean
//!
//! A gate fails when more than its share of a fixture's pixels differ from the reference by more
//! than its tolerance in any channel, or when the fixture's capture failed. The limits are per
//! platform because the rasterizer under GPUI is per platform; see `GATES`. On Linux the executor
//! runs on a **software** Vulkan driver (Mesa lavapipe), so a pass proves that the Prim stream's
//! coordinates, colours, paint order and blend arithmetic reach a real GPUI window unchanged, and
//! that antialiased edges stay within the measured envelope (roughly: strokes keep their width to
//! about half a pixel). It does **not** prove hardware-GPU behaviour: rasterization rules, MSAA
//! sample patterns, text rasterization and gamma may differ from DWM/WARP and from real GPUs, and
//! the text fixture is not gated because the two sides draw different font faces.
//!
//! It does not exercise clipping either. Every fixture is painted through
//! `GpuiChartRenderer::paint_prims`, a bare Prim layer with no frame and no clip, and clips are
//! lowered only on the frame path. The frame-level pane matrix is the harness that covers clipping:
//! `gpui_pane_capture` paints the engine's pane frame through `paint_frame`, and
//! `examples/web_demo/tests/gpui-webgpu-matrix.spec.mjs` compares it with the presented WebGPU frame.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "linux"))]
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use aeris_charts_render_gpui::fixtures::{self, Fixture};
use aeris_charts_render_gpui::{AerisViewport, GpuiChartRenderer};
use gpui::{
    canvas, div, prelude::*, px, size, App, Bounds, Context, Entity, Render, Window, WindowBounds,
    WindowOptions,
};
use gpui_platform::application;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
#[path = "support/x11_capture.rs"]
mod x11_capture;

/// A unique window title, so the capture script can find exactly this window.
const WINDOW_TITLE: &str = "aeris_charts-pixel-parity-harness";

/// Frames to paint before capturing, so the swapchain has certainly presented the fixture.
const WARMUP_FRAMES: u64 = 12;

/// Wall-clock budget for the whole run, from process start to the final report.
///
/// A normal Linux run (xvfb, lavapipe, dev profile, build excluded) takes 9 to 10.5 s for the eight
/// fixtures, measured as the wall clock of the documented command with `xvfb-run` and cargo startup
/// included. The budget is more than eleven times the longest: room for a slower runner, and for
/// the Windows path, which starts PowerShell for every capture and has not been timed. A run that
/// exceeds it is recorded as a failure, `results.json` is written, and the process exits with
/// code 1. Without an X display GPUI opens a headless window that paints once and never again, so
/// no frame-driven check could end the run.
const DEADLINE: Duration = Duration::from_secs(120);

/// Where the harness is in the render/capture cycle for the current fixture.
///
/// The capture **must not** run on the UI thread: `PrintWindow` posts `WM_PRINT` to the target
/// window and waits for it to be serviced, so calling it from inside the paint callback deadlocks
/// against our own message loop. It therefore runs on a worker thread while the main thread keeps
/// painting the same fixture, which also guarantees the window still shows that fixture when DWM
/// composes the capture.
enum Phase {
    /// Painting the fixture; capture once the counter passes [`WARMUP_FRAMES`].
    Warmup(u64),
    /// A capture is in flight on a worker thread.
    Capturing(Receiver<String>),
}

/// What the final report is built from. The deadline thread shares it with the harness, so a run
/// that stops advancing still ends with a `results.json` and a failing exit code.
struct Evidence {
    rows: Vec<Row>,
    /// The GPU adapter GPUI selected, recorded from the first painted frame.
    adapter: String,
    /// `None` until the final report ran (by the harness or by the deadline), then whether it passed.
    passed: Option<bool>,
}

fn lock(evidence: &Mutex<Evidence>) -> MutexGuard<'_, Evidence> {
    evidence.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Harness {
    fixtures: Vec<Fixture>,
    /// Index of the fixture currently being displayed.
    current: usize,
    renderer: GpuiChartRenderer,
    phase: Phase,
    out_dir: PathBuf,
    tolerance: u8,
    evidence: Arc<Mutex<Evidence>>,
    /// Set once the last fixture has been captured.
    done: bool,
}

struct Row {
    name: String,
    attribution: String,
    width: u32,
    height: u32,
    differing: u32,
    max_delta: u8,
    /// Pixels whose largest channel delta is above each of [`EDGES`] (the residual by magnitude).
    exceed: [u32; EDGES.len()],
    total: u32,
    /// Mean absolute channel error of the GPUI image against the reference shifted by each of
    /// [`SHIFTS`]; `None` when no comparison was made.
    alignment: Option<[f64; SHIFTS.len()]>,
    /// Whether this run captured the GPUI image. A row without it has no GPUI or diff hash.
    captured: bool,
    note: String,
}

impl Row {
    /// A fixture without a comparison. The note says why, and every gate on the fixture fails with it.
    fn unmeasured(
        name: &str,
        attribution: &str,
        (width, height): (u32, u32),
        note: String,
    ) -> Self {
        Self {
            name: name.into(),
            attribution: attribution.into(),
            width,
            height,
            differing: 0,
            max_delta: 0,
            exceed: [0; EDGES.len()],
            total: 0,
            alignment: None,
            captured: false,
            note,
        }
    }
}

/// Magnitude edges of the residual, in channel values: 0 is "any difference", 1 is blend rounding,
/// and 4, 16 and 64 separate interpolation error, edge-coverage ramps and wrong coverage or colour.
const EDGES: [u8; 5] = [0, 1, 4, 16, 64];

/// For each of [`EDGES`], how many pixels differ by more than it in any channel.
fn pixels_over(a: &tiny_skia::Pixmap, b: &tiny_skia::Pixmap) -> [u32; EDGES.len()] {
    let mut over = [0; EDGES.len()];
    let (a_pixels, _) = a.data().as_chunks::<4>();
    let (b_pixels, _) = b.data().as_chunks::<4>();
    for (pa, pb) in a_pixels.iter().zip(b_pixels) {
        let delta = pa.iter().zip(pb).map(|(x, y)| x.abs_diff(*y)).max();
        for (count, edge) in over.iter_mut().zip(EDGES) {
            *count += u32::from(delta > Some(edge));
        }
    }
    over
}

/// Offsets `(dx, dy)` of the reference for the alignment check: none, then one pixel each way in x
/// and in y. The first entry must stay `(0, 0)`.
const SHIFTS: [(i32, i32); 5] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)];

/// Mean absolute channel error between `a` and `b` shifted by `(dx, dy)`, where the shifted `b` at
/// `(x, y)` is `b` at `(x + dx, y + dy)`. It averages the interior that every shift of at most one
/// pixel shares, so every shift is measured over the same pixels.
fn mean_abs_error_shifted(a: &tiny_skia::Pixmap, b: &tiny_skia::Pixmap, dx: i32, dy: i32) -> f64 {
    let (width, height) = (a.width() as usize, a.height() as usize);
    let (pa, pb) = (a.data(), b.data());
    let mut sum = 0u64;
    for y in 1..height.saturating_sub(1) {
        let by = y.saturating_add_signed(dy as isize);
        for x in 1..width.saturating_sub(1) {
            let bx = x.saturating_add_signed(dx as isize);
            let (ia, ib) = ((y * width + x) * 4, (by * width + bx) * 4);
            sum += pa[ia..ia + 4]
                .iter()
                .zip(&pb[ib..ib + 4])
                .map(|(p, q)| u64::from(p.abs_diff(*q)))
                .sum::<u64>();
        }
    }
    let samples = width.saturating_sub(2).max(1) * height.saturating_sub(2).max(1) * 4;
    sum as f64 / samples as f64
}

/// The path of one of a fixture's images; `kind` is `gpui`, `native` or `diff`.
fn image_path(dir: &Path, fixture: &str, kind: &str) -> PathBuf {
    dir.join(format!("{fixture}_{kind}.png"))
}

/// Delete a fixture's images from an earlier run in the same directory (see
/// [`remove_stale_evidence`]).
fn remove_stale_images(dir: &Path, fixture: &str) -> Result<(), String> {
    for kind in ["gpui", "native", "diff"] {
        let path = image_path(dir, fixture, kind);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("could not delete {}: {error}", path.display()));
            }
        }
    }
    Ok(())
}

/// Delete everything an earlier run left in `dir`: `results.json` and every fixture's images.
fn remove_stale_evidence(dir: &Path) -> Result<(), String> {
    let results = dir.join("results.json");
    match std::fs::remove_file(&results) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not delete {}: {error}", results.display())),
    }
    for fixture in fixtures::all(1.0) {
        remove_stale_images(dir, fixture.name)?;
    }
    Ok(())
}

impl Harness {
    fn new(dpr: f32, out_dir: PathBuf, tolerance: u8, evidence: Arc<Mutex<Evidence>>) -> Self {
        Self {
            fixtures: fixtures::all(dpr),
            current: 0,
            renderer: GpuiChartRenderer::new(),
            phase: Phase::Warmup(0),
            out_dir,
            tolerance,
            evidence,
            done: false,
        }
    }

    /// Rasterize the reference and kick off the window capture on a worker thread.
    fn spawn_capture(&mut self, #[cfg(target_os = "linux")] window: &Window) -> Receiver<String> {
        let (tx, rx) = mpsc::channel();
        let f = &self.fixtures[self.current];
        let gpui_png = image_path(&self.out_dir, f.name, "gpui");
        let native_png = image_path(&self.out_dir, f.name, "native");

        // Reference: Aeris's existing native rasterizer over the identical prim list.
        let canvas =
            aeris_charts_native::render_prims(f.width, f.height, f.background, &f.prims, &f.points);
        if let Err(e) = canvas.save_png(native_png.to_str().unwrap_or_default()) {
            let _ = tx.send(format!("ERR native-render: {e}"));
            return rx;
        }

        // GPUI: read back what the window actually presented, off the UI thread.
        #[cfg(target_os = "linux")]
        {
            // The window id is only reachable from the UI thread; the X11 read itself is not.
            let window_id = x11_capture::window_id(window);
            let expected = (f.width, f.height);
            let out = gpui_png.clone();
            std::thread::spawn(move || {
                let note = window_id
                    .and_then(|id| x11_capture::capture(id, expected, &out))
                    .unwrap_or_else(|e| format!("ERR {e}"));
                let _ = tx.send(note);
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/capture_window.ps1");
            let out = gpui_png.clone();
            std::thread::spawn(move || {
                let output = Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-File",
                        script.to_str().unwrap_or_default(),
                        "-Title",
                        WINDOW_TITLE,
                        "-Out",
                        out.to_str().unwrap_or_default(),
                    ])
                    .output();
                let note = match &output {
                    Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
                    Err(e) => format!("ERR capture-spawn: {e}"),
                };
                let _ = tx.send(note);
            });
        }
        rx
    }

    /// Diff the captured fixture against the reference and record the row.
    fn finish_capture(&mut self, note: String) {
        let f = &self.fixtures[self.current];
        let row = if note.starts_with("OK") {
            compare(f, &self.out_dir, self.tolerance)
        } else {
            Row::unmeasured(
                f.name,
                f.attribution,
                (f.width, f.height),
                format!("capture failed: {note}"),
            )
        };
        lock(&self.evidence).rows.push(row);
    }
}

/// Diff the captured GPUI image of `f` against its reference with Aeris's own comparator, at the
/// requested tolerance, and measure how well the two are aligned.
fn compare(f: &Fixture, dir: &Path, tolerance: u8) -> Row {
    let load = |kind: &str| {
        let path = image_path(dir, f.name, kind);
        aeris_charts_native::load_png(path.to_str().unwrap_or_default())
    };
    // The GPUI image exists, so the row keeps its hashes even when the comparison cannot be made.
    let unmeasured = |note: String| Row {
        captured: true,
        ..Row::unmeasured(f.name, f.attribution, (f.width, f.height), note)
    };
    let (a, b) = match (load("gpui"), load("native")) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            return unmeasured(format!(
                "png load failed: gpui={:?} native={:?}",
                a.err(),
                b.err()
            ))
        }
    };
    if a.width() != b.width() || a.height() != b.height() {
        return unmeasured(format!(
            "size mismatch: captured {}x{}, expected {}x{} — the window client area \
             is not the fixture size, so no comparison was made",
            a.width(),
            a.height(),
            b.width(),
            b.height()
        ));
    }
    write_diff_image(&a, &b, &image_path(dir, f.name, "diff"));
    let Some(d) = aeris_charts_native::diff_pixmaps(&a, &b, tolerance) else {
        return unmeasured("diff_pixmaps rejected the pair".into());
    };
    let mut alignment = [0.0; SHIFTS.len()];
    for (error, (dx, dy)) in alignment.iter_mut().zip(SHIFTS) {
        *error = mean_abs_error_shifted(&a, &b, dx, dy);
    }
    Row {
        name: f.name.into(),
        attribution: f.attribution.into(),
        width: a.width(),
        height: a.height(),
        differing: d.differing_pixels,
        max_delta: d.max_channel_delta,
        exceed: pixels_over(&a, &b),
        total: d.total_pixels,
        alignment: Some(alignment),
        captured: true,
        note: String::new(),
    }
}

/// Print the table, write `results.json`, then evaluate [`GATES`] and [`ALIGNED`], and record the
/// outcome in `evidence`.
fn report(out_dir: &Path, tolerance: u8, evidence: &mut Evidence) {
    println!("\n=== aeris_charts_render_gpui pixel parity ===");
    println!("reference: aeris_charts_native (tiny-skia) over the identical Prim list");
    println!("gpui     : window client area captured through {CAPTURE_SOURCE}");
    println!("adapter  : {}", evidence.adapter);
    println!("tolerance: {tolerance} (per channel)\n");
    println!(
        "{:<16} {:>11} {:>10} {:>9} {:>8} {:>7} {:>7} {:>7} {:>7}  attribution / note",
        "fixture", "size", "differing", "% of px", "maxdelta", ">1", ">4", ">16", ">64"
    );
    for r in &evidence.rows {
        if !r.note.is_empty() {
            println!(
                "{:<16} {:>11} {:>10} {:>9} {:>8} {:>7} {:>7} {:>7} {:>7}  {}",
                r.name, "-", "-", "-", "-", "-", "-", "-", "-", r.note
            );
            continue;
        }
        let pct = if r.total == 0 {
            0.0
        } else {
            r.differing as f64 * 100.0 / r.total as f64
        };
        println!(
            "{:<16} {:>11} {:>10} {:>8.3}% {:>8} {:>7} {:>7} {:>7} {:>7}  {}",
            r.name,
            format!("{}x{}", r.width, r.height),
            r.differing,
            pct,
            r.max_delta,
            r.exceed[1],
            r.exceed[2],
            r.exceed[3],
            r.exceed[4],
            r.attribution
        );
    }
    println!("\nartifacts: {}", out_dir.display());
    let mut failures = Vec::new();
    // Evidence first: a failing gate must still leave `results.json` and the images behind.
    if let Err(error) = write_results_json(out_dir, &evidence.rows, &evidence.adapter) {
        failures.push(format!("results.json was not written: {error}"));
    }

    // The crisp-rect fixture is the one that can legitimately be held to zero.
    if let Some(crisp) = evidence.rows.iter().find(|r| r.name == "crisp_rects") {
        if crisp.note.is_empty() {
            println!(
                "\ncrisp-rect gate: {} ({} differing pixels, max channel delta {})",
                if crisp.differing == 0 { "PASS" } else { "FAIL" },
                crisp.differing,
                crisp.max_delta
            );
        } else {
            println!("\ncrisp-rect gate: NOT MEASURED — {}", crisp.note);
        }
    }
    println!("\ngates ({GATE_SET}):");
    for gate in GATES {
        let failure = check_gate(gate, &evidence.rows);
        println!(
            "  {} {:<16} pixels off by more than {} <= {}% of the fixture",
            if failure.is_none() { "PASS" } else { "FAIL" },
            gate.fixture,
            gate.tolerance,
            gate.max_pct
        );
        failures.extend(failure);
    }
    println!(
        "\nalignment ({}): mean absolute channel error, no shift against the reference shifted by 1 px",
        ALIGNED.len()
    );
    for fixture in ALIGNED {
        match check_alignment(fixture, &evidence.rows) {
            Ok(detail) => println!("  PASS {fixture:<16} {detail}"),
            Err(failure) => {
                println!("  FAIL {fixture:<16} {failure}");
                failures.push(format!("{fixture} alignment: {failure}"));
            }
        }
    }
    for failure in &failures {
        eprintln!("gate failure: {failure}");
    }
    evidence.passed = Some(failures.is_empty());
}

/// A hard limit on one fixture's residual against the reference rasterizer: at most `max_pct`
/// percent of its pixels may differ by more than `tolerance` (one of [`EDGES`]) in any channel.
struct Gate {
    fixture: &'static str,
    tolerance: u8,
    max_pct: f64,
}

#[cfg(target_os = "linux")]
const CAPTURE_SOURCE: &str = "the X server (GetImage on GPUI's X11 window)";
#[cfg(not(target_os = "linux"))]
const CAPTURE_SOURCE: &str = "DWM (PrintWindow)";

#[cfg(target_os = "linux")]
const GATE_SET: &str = "Linux, software Vulkan";
#[cfg(not(target_os = "linux"))]
const GATE_SET: &str = "crosshair icon";

/// Limits for the Linux software-Vulkan stack (Mesa lavapipe 25.2.8, LLVM 20.1.2).
///
/// Measured at scale factors 1.0 and 1.5, three consecutive runs each: every count and every GPUI
/// image hash was identical run to run (and with `LP_NUM_THREADS=1`), so the limits carry no noise
/// allowance. Exact where the cause allows it, otherwise the measurement at scale 1.0 (the larger
/// residual; the edge share falls as the scale grows) plus 25% headroom for a different Mesa/LLVM
/// build, which cannot be measured on one machine. Percentages are of the fixture's pixels.
///
/// | fixture          | measured at 1.0                         | limit                    |
/// |------------------|-----------------------------------------|--------------------------|
/// | crisp_rects      | 0 px differ                             | exact                    |
/// | crosshair_action | 0 px differ                             | none above 1 (contract)  |
/// | translucent      | 4.917% differ, every one by exactly 1   | none above 1             |
/// | gradients        | >4: 0.235%, >16: 0.056% (edge), >64: 0  | >4: 0.3%, >64: none      |
/// | opaque_aa        | any: 1.582%, >64: 0.546%                | any: 2.0%, >64: 0.7%     |
/// | tessellated      | any: 1.850%, >64: 1.197%                | any: 2.4%, >64: 1.5%     |
/// | curved_brushes   | any: 3.783%, >64: 2.282%                | any: 4.8%, >64: 2.9%     |
///
/// `text` is reported but its residual is deliberately not bounded (its 100% limit only requires
/// the capture to succeed): GPUI resolves "sans-serif" through fontconfig while the native
/// reference resolves it through `fontdb`, so the two draw different faces and the residual
/// measures the host's font choice, not the executor.
///
/// A percentage of pixels is a weak test of position: a thin antialiased stroke shifted by one pixel
/// can stay inside its share, so [`ALIGNED`] adds a second check for the antialiased fixtures.
#[cfg(target_os = "linux")]
const GATES: &[Gate] = &[
    Gate {
        fixture: "text",
        tolerance: 0,
        max_pct: 100.0,
    },
    Gate {
        fixture: "crisp_rects",
        tolerance: 0,
        max_pct: 0.0,
    },
    Gate {
        fixture: "crosshair_action",
        tolerance: 1,
        max_pct: 0.0,
    },
    Gate {
        fixture: "translucent",
        tolerance: 1,
        max_pct: 0.0,
    },
    Gate {
        fixture: "gradients",
        tolerance: 4,
        max_pct: 0.3,
    },
    Gate {
        fixture: "gradients",
        tolerance: 64,
        max_pct: 0.0,
    },
    Gate {
        fixture: "opaque_aa",
        tolerance: 0,
        max_pct: 2.0,
    },
    Gate {
        fixture: "opaque_aa",
        tolerance: 64,
        max_pct: 0.7,
    },
    Gate {
        fixture: "tessellated",
        tolerance: 0,
        max_pct: 2.4,
    },
    Gate {
        fixture: "tessellated",
        tolerance: 64,
        max_pct: 1.5,
    },
    Gate {
        fixture: "curved_brushes",
        tolerance: 0,
        max_pct: 4.8,
    },
    Gate {
        fixture: "curved_brushes",
        tolerance: 64,
        max_pct: 2.9,
    },
];
/// The crosshair icon must match native rendering to one channel value of blending rounding.
#[cfg(not(target_os = "linux"))]
const GATES: &[Gate] = &[Gate {
    fixture: "crosshair_action",
    tolerance: 1,
    max_pct: 0.0,
}];

/// The antialiased fixtures held to the alignment check: the mean absolute channel error of GPUI's
/// image against the reference at no shift must be lower than against the reference shifted by one
/// pixel in x or y, in either direction ([`SHIFTS`]). Every limit in [`GATES`] counts pixels, and
/// a one-pixel shift of the reference in x stays inside both `tessellated` and both `curved_brushes`
/// limits (only `opaque_aa` fails it); this check fails every one of these fixtures for a shift in
/// any of the four directions. Measured at scale 1.0 on the calibration stack, as mean absolute
/// channel error at no shift against the closest shifted reference:
///
/// | fixture          | no shift | closest 1 px shift |
/// |------------------|----------|--------------------|
/// | opaque_aa        | 0.3816   | 0.8287             |
/// | tessellated      | 0.5910   | 0.7914             |
/// | curved_brushes   | 1.6159   | 1.8025             |
///
/// At scale 1.5 the same ordering holds: 0.3446 against 0.6454, 0.3841 against 0.5179 and 1.0638
/// against 1.1912.
///
/// There is no limit to calibrate and so no headroom: the comparison is between two measurements of
/// the same pair of images. It is not run on Windows, where no antialiasing limit is gated.
#[cfg(target_os = "linux")]
const ALIGNED: &[&str] = &["opaque_aa", "tessellated", "curved_brushes"];
#[cfg(not(target_os = "linux"))]
const ALIGNED: &[&str] = &[];

/// The failure message when `gate` does not hold; a missing or failed capture fails every gate on
/// its fixture, so an unmeasured fixture can never pass.
fn check_gate(gate: &Gate, results: &[Row]) -> Option<String> {
    let Some(row) = results.iter().find(|row| row.name == gate.fixture) else {
        return Some(format!("{}: no result row was produced", gate.fixture));
    };
    if !row.note.is_empty() {
        return Some(format!("{}: {}", gate.fixture, row.note));
    }
    let edge = EDGES
        .iter()
        .position(|edge| *edge == gate.tolerance)
        .expect("a gate tolerance is one of EDGES");
    let over = row.exceed[edge];
    let pct = f64::from(over) * 100.0 / f64::from(row.total.max(1));
    (pct > gate.max_pct).then(|| {
        format!(
            "{}: {over} pixels ({pct:.3}%) differ by more than {} (limit {}%)",
            gate.fixture, gate.tolerance, gate.max_pct
        )
    })
}

/// `Ok` with the measured errors when `fixture` is better aligned with its reference at no shift
/// than at every one-pixel shift, `Err` with the failure otherwise. A missing, failed or non-finite
/// measurement fails.
fn check_alignment(fixture: &str, results: &[Row]) -> Result<String, String> {
    let row = results
        .iter()
        .find(|row| row.name == fixture)
        .ok_or("no result row was produced")?;
    if !row.note.is_empty() {
        return Err(row.note.clone());
    }
    let errors = row.alignment.ok_or("no alignment was measured")?;
    let closest = (1..SHIFTS.len())
        .min_by(|a, b| errors[*a].total_cmp(&errors[*b]))
        .expect("SHIFTS lists shifted references after the unshifted one");
    let (dx, dy) = SHIFTS[closest];
    if errors[0] < errors[closest] {
        Ok(format!(
            "{:.4} at no shift, {:.4} at the closest shift ({dx:+},{dy:+})",
            errors[0], errors[closest]
        ))
    } else {
        Err(format!(
            "mean absolute error {:.4} at no shift is not below {:.4} with the reference shifted by \
             ({dx:+},{dy:+}) px, so GPUI's geometry is not aligned with the reference",
            errors[0], errors[closest]
        ))
    }
}

/// Write a human-visible diff image: differing pixels in magenta over a dimmed reference.
fn write_diff_image(a: &tiny_skia::Pixmap, b: &tiny_skia::Pixmap, path: &std::path::Path) {
    let Some(mut out) = tiny_skia::Pixmap::new(a.width(), a.height()) else {
        return;
    };
    let (pa, pb) = (a.data(), b.data());
    let dst = out.pixels_mut();
    for i in 0..(a.width() * a.height()) as usize {
        let differs = (0..4).any(|c| pa[i * 4 + c] != pb[i * 4 + c]);
        let px = if differs {
            tiny_skia::PremultipliedColorU8::from_rgba(0xff, 0x00, 0xff, 0xff)
        } else {
            let g = pb[i * 4] / 3 + 170;
            tiny_skia::PremultipliedColorU8::from_rgba(g, g, g, 0xff)
        };
        if let Some(px) = px {
            dst[i] = px;
        }
    }
    let _ = out.save_png(path);
}

fn sha256_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// Hash the decoded premultiplied RGBA bytes used by `diff_pixmaps`.
///
/// PNG-file hashes can differ for pixel-identical images because their encoders choose different
/// chunking/compression. This is the canonical visual hash required by the parity gate.
fn rgba_sha256_file(path: &Path) -> Option<String> {
    let pixmap = aeris_charts_native::load_png(path.to_str().unwrap_or_default()).ok()?;
    Some(sha256_hex(pixmap.data()))
}

/// Write `results.json`: one object per fixture, serialized by `serde_json` so that every note and
/// adapter string stays valid JSON. An image that does not exist, and the GPUI and diff images of a
/// row whose capture failed, have a `null` hash.
fn write_results_json(dir: &Path, rows: &[Row], adapter: &str) -> Result<(), String> {
    let entries: Vec<Value> = rows
        .iter()
        .map(|r| {
            let (gpui, native, diff) = (
                image_path(dir, &r.name, "gpui"),
                image_path(dir, &r.name, "native"),
                image_path(dir, &r.name, "diff"),
            );
            let if_captured = |path: &Path, hash: fn(&Path) -> Option<String>| {
                r.captured.then(|| hash(path)).flatten()
            };
            let pixels_over: Map<String, Value> = EDGES
                .iter()
                .zip(r.exceed)
                .map(|(edge, count)| (edge.to_string(), json!(count)))
                .collect();
            let mean_abs_error: Option<Map<String, Value>> = r.alignment.map(|errors| {
                SHIFTS
                    .iter()
                    .zip(errors)
                    .map(|((dx, dy), error)| (format!("{dx},{dy}"), json!(error)))
                    .collect()
            });
            json!({
                "fixture": r.name,
                "attribution": r.attribution,
                "gpui_adapter": adapter,
                "width": r.width,
                "height": r.height,
                "differing_pixels": r.differing,
                "max_channel_delta": r.max_delta,
                "total_pixels": r.total,
                "pixels_over": pixels_over,
                "mean_abs_error_by_shift": mean_abs_error,
                "gpui_rgba_sha256": if_captured(&gpui, rgba_sha256_file),
                "native_rgba_sha256": rgba_sha256_file(&native),
                "gpui_png_sha256": if_captured(&gpui, sha256_file),
                "native_png_sha256": sha256_file(&native),
                "diff_png_sha256": if_captured(&diff, sha256_file),
                "note": r.note,
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&entries).map_err(|error| error.to_string())?;
    text.push('\n');
    let path = dir.join("results.json");
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))
}

impl Render for Harness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.done {
            cx.quit();
            return div();
        }
        let entity: Entity<Harness> = cx.entity();
        window.request_animation_frame();

        div().size_full().child(
            canvas(
                move |bounds: Bounds<gpui::Pixels>, _window, _cx| bounds,
                move |bounds: Bounds<gpui::Pixels>, _prep, window, cx| {
                    entity.update(cx, |h: &mut Harness, cx| {
                        let sf = window.scale_factor();
                        // Paint the current fixture.
                        let f = &h.fixtures[h.current];
                        let plan_prims = f.prims.clone();
                        let plan_points = f.points.clone();
                        let viewport = AerisViewport::from_bounds(
                            bounds.origin.x.into(),
                            bounds.origin.y.into(),
                            bounds.size.width.into(),
                            bounds.size.height.into(),
                        );
                        h.renderer
                            .paint_prims(&plan_prims, &plan_points, viewport, sf, window, cx);
                        {
                            let mut evidence = lock(&h.evidence);
                            if evidence.adapter.is_empty() {
                                evidence.adapter = describe_adapter(window);
                                println!("harness: gpui adapter {}", evidence.adapter);
                            }
                        }

                        // Advance the capture state machine. The main thread keeps painting the
                        // same fixture throughout, so the window content stays valid for DWM.
                        match &h.phase {
                            Phase::Warmup(n) if *n > WARMUP_FRAMES => {
                                let rx = h.spawn_capture(
                                    #[cfg(target_os = "linux")]
                                    window,
                                );
                                h.phase = Phase::Capturing(rx);
                            }
                            Phase::Warmup(n) => h.phase = Phase::Warmup(n + 1),
                            Phase::Capturing(rx) => {
                                let note = match rx.try_recv() {
                                    Ok(note) => Some(note),
                                    Err(TryRecvError::Empty) => None,
                                    // The worker ended without sending (it panicked): a failed
                                    // capture, not a result still to come.
                                    Err(TryRecvError::Disconnected) => {
                                        Some("ERR the capture worker ended without a result".into())
                                    }
                                };
                                if let Some(note) = note {
                                    h.finish_capture(note);
                                    h.phase = Phase::Warmup(0);
                                    if h.current + 1 < h.fixtures.len() {
                                        h.current += 1;
                                    } else {
                                        report(&h.out_dir, h.tolerance, &mut lock(&h.evidence));
                                        h.done = true;
                                    }
                                }
                            }
                        }
                    });
                },
            )
            .size_full(),
        )
    }
}

/// Start the thread that ends a run which stops advancing: after [`DEADLINE`] it records every
/// fixture without a result as a failure, reports (writing `results.json`) and exits with code 1.
/// It does nothing when the harness has already reported.
fn spawn_deadline(evidence: Arc<Mutex<Evidence>>, out_dir: PathBuf, tolerance: u8) {
    std::thread::spawn(move || {
        std::thread::sleep(DEADLINE);
        let mut evidence = lock(&evidence);
        if evidence.passed.is_some() {
            return;
        }
        // Fixtures are captured in order, so the rows present are the first ones.
        let roster = fixtures::all(1.0);
        let captured = evidence.rows.len();
        eprintln!(
            "harness: {} s deadline reached with {captured} of {} fixtures captured; the window \
             stopped painting (on Linux a missing X display does that) or a capture never returned",
            DEADLINE.as_secs(),
            roster.len()
        );
        for f in roster.iter().skip(captured) {
            evidence.rows.push(Row::unmeasured(
                f.name,
                f.attribution,
                (0, 0),
                format!(
                    "not measured: the {} s harness deadline passed before this fixture was captured",
                    DEADLINE.as_secs()
                ),
            ));
        }
        report(&out_dir, tolerance, &mut evidence);
        std::process::exit(1);
    });
}

fn main() {
    let out_dir = PathBuf::from(
        std::env::var("AERIS_CHARTS_PARITY_OUT").unwrap_or_else(|_| "target/pixel_parity".into()),
    );
    std::fs::create_dir_all(&out_dir).expect("output directory is creatable");
    // One cleanup point, before anything can fail: no run, however early it ends (a missing display,
    // a GPUI panic at startup, the deadline), may leave an earlier run's images, hashes or
    // `results.json` in the directory as its own evidence.
    if let Err(error) = remove_stale_evidence(&out_dir) {
        eprintln!("harness: {error}");
        std::process::exit(1);
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!(
            "harness: DISPLAY is not set; the Linux capture reads the window from an X server. \
             Run under a display with WAYLAND_DISPLAY unset, for example `xvfb-run -a` (see the \
             command in the header)"
        );
        std::process::exit(1);
    }
    let tolerance: u8 = std::env::var("AERIS_CHARTS_PARITY_TOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let evidence = Arc::new(Mutex::new(Evidence {
        rows: Vec::new(),
        adapter: String::new(),
        passed: None,
    }));
    spawn_deadline(Arc::clone(&evidence), out_dir.clone(), tolerance);

    let harness_evidence = Arc::clone(&evidence);
    application().run(move |cx: &mut App| {
        // A GPUI platform built without its text system (macOS `font-kit`) quietly substitutes a
        // no-op one, so the text fixture would compare an empty window. Fail before capturing.
        if cx.text_system().all_font_names().is_empty() {
            eprintln!(
                "pixel_parity: GPUI has no text system, so no text would render \
                 (on macOS the GPUI platform needs its `font-kit` feature)"
            );
            std::process::exit(1);
        }
        // The window's client area must be exactly the fixture's logical size, so the captured
        // pixels are exactly the surface the adapter painted.
        let bounds = Bounds::centered(
            None,
            size(px(fixtures::LOGICAL_W), px(fixtures::LOGICAL_H)),
            cx,
        );
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(WINDOW_TITLE.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let scale = window.scale_factor();
                println!("harness: window scale factor {scale}");
                cx.new(|_| {
                    Harness::new(
                        scale,
                        out_dir.clone(),
                        tolerance,
                        Arc::clone(&harness_evidence),
                    )
                })
            },
        )
        .expect("the harness window opens");
        cx.activate(true);
    });
    let passed = lock(&evidence).passed;
    match passed {
        Some(true) => {}
        Some(false) => std::process::exit(1),
        None => {
            eprintln!("harness: GPUI stopped before the final report; no result was produced");
            std::process::exit(1);
        }
    }
}

/// The adapter GPUI selected, as Vulkan reports it, so a result says what rasterized it.
fn describe_adapter(window: &Window) -> String {
    match window.gpu_specs() {
        Some(specs) => format!(
            "{} (driver {} {}, software emulated: {})",
            specs.device_name, specs.driver_name, specs.driver_info, specs.is_software_emulated
        ),
        None => "unknown (GPUI reports no GPU specs on this platform)".into(),
    }
}
