//! Linux window capture shared by `pixel_parity` and `gpui_pane_capture`.
//!
//! GPUI exposes no framebuffer readback, so on Windows the examples read the composed window
//! through DWM (`tools/capture_window.ps1`). On Linux the same question is answered by asking the
//! X server for the pixels of GPUI's own window: [`window_id`] takes the X11 window id from GPUI's
//! `raw-window-handle` support and [`capture`] reads that window's client area with `GetImage`.
//! Nothing is injected into GPUI, no device is shared and Aeris does not re-rasterize the chart
//! for this side, so it reads exactly what the swapchain presented. (On a software Vulkan driver
//! that swapchain is itself CPU-rasterized, which is why a result is specific to that driver.)
//!
//! Preconditions, each checked and reported instead of assumed:
//! - GPUI runs on X11 (`DISPLAY` set, `WAYLAND_DISPLAY` unset); a Wayland window has no X11 id.
//! - The window is mapped and fully on the screen: the X server clamps a window to its screen and
//!   `GetImage` fails with `BadMatch` for off-screen parts, so run on a virtual display at least
//!   as large as the window, for example `xvfb-run -a -s "-screen 0 2560x1600x24"`.
//! - The client area is exactly the expected device-pixel size, or no comparison is made.
//! - The window visual is 32 bits per pixel with the usual `0xff0000`/`0xff00`/`0xff` masks.
//!
//! Depth 24 windows carry no alpha, so alpha is reported as opaque; depth 32 windows keep the
//! alpha the swapchain wrote.

use std::path::Path;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, ImageOrder, MapState};
use x11rb::rust_connection::RustConnection;

/// The X11 id of the window GPUI opened. Must run on the UI thread, where `window` is reachable.
pub fn window_id(window: &gpui::Window) -> Result<u32, String> {
    // `Window` also has an inherent `window_handle` (GPUI's own handle), so name the trait.
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| format!("GPUI exposes no native window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::Xcb(xcb) => Ok(xcb.window.get()),
        other => Err(format!(
            "the GPUI window is not an X11 window ({other:?}); run with DISPLAY set and \
             WAYLAND_DISPLAY unset so GPUI selects X11"
        )),
    }
}

/// Read the client area of X11 window `window`, require it to be `expected` device pixels, and
/// write it as a PNG at `out`. Returns `OK <width> <height> depth=<depth>`, the same contract as
/// `tools/capture_window.ps1`, so callers handle both platforms alike.
pub fn capture(window: u32, expected: (u32, u32), out: &Path) -> Result<String, String> {
    let (conn, _screen) = RustConnection::connect(None)
        .map_err(|error| format!("cannot connect to the X server named by DISPLAY: {error}"))?;
    let request = |what: &str, error: &dyn std::fmt::Display| format!("X11 {what} failed: {error}");

    let attributes = conn
        .get_window_attributes(window)
        .map_err(|e| request("GetWindowAttributes", &e))?
        .reply()
        .map_err(|e| request("GetWindowAttributes", &e))?;
    if attributes.map_state != MapState::VIEWABLE {
        return Err(format!(
            "window {window:#x} is not viewable (map state {:?}), so the X server holds no pixels for it",
            attributes.map_state
        ));
    }
    let geometry = conn
        .get_geometry(window)
        .map_err(|e| request("GetGeometry", &e))?
        .reply()
        .map_err(|e| request("GetGeometry", &e))?;
    let size = (u32::from(geometry.width), u32::from(geometry.height));
    if size != expected {
        return Err(format!(
            "window client area is {}x{}, expected {}x{} device pixels, so no comparison was made \
             (the X server clamps a window to its screen: use a larger virtual screen)",
            size.0, size.1, expected.0, expected.1
        ));
    }

    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window,
            0,
            0,
            geometry.width,
            geometry.height,
            !0,
        )
        .map_err(|e| request("GetImage", &e))?
        .reply()
        .map_err(|e| request("GetImage", &e))?;

    let setup = conn.setup();
    let format = setup
        .pixmap_formats
        .iter()
        .find(|format| format.depth == image.depth)
        .ok_or_else(|| {
            format!(
                "the X server lists no pixmap format for depth {}",
                image.depth
            )
        })?;
    if format.bits_per_pixel != 32 || format.scanline_pad != 32 {
        return Err(format!(
            "unsupported pixmap format for depth {}: {} bits per pixel, scanline pad {}",
            image.depth, format.bits_per_pixel, format.scanline_pad
        ));
    }
    let visual = setup
        .roots
        .iter()
        .flat_map(|root| &root.allowed_depths)
        .flat_map(|depth| &depth.visuals)
        .find(|visual| visual.visual_id == image.visual)
        .ok_or_else(|| format!("visual {:#x} is not in the connection setup", image.visual))?;
    if setup.image_byte_order != ImageOrder::LSB_FIRST
        || (visual.red_mask, visual.green_mask, visual.blue_mask) != (0xff_0000, 0xff00, 0xff)
    {
        return Err(format!(
            "unsupported visual layout: byte order {:?}, masks {:#x}/{:#x}/{:#x} (expected BGRA)",
            setup.image_byte_order, visual.red_mask, visual.green_mask, visual.blue_mask
        ));
    }

    let (width, height) = size;
    let expected_bytes = width as usize * height as usize * 4;
    if image.data.len() != expected_bytes {
        return Err(format!(
            "GetImage returned {} bytes, expected {expected_bytes}",
            image.data.len()
        ));
    }
    let opaque = image.depth != 32;
    let mut rgba = Vec::with_capacity(expected_bytes);
    for pixel in image.data.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[
            pixel[2],
            pixel[1],
            pixel[0],
            if opaque { 0xff } else { pixel[3] },
        ]);
    }
    let pixmap = tiny_skia::IntSize::from_wh(width, height)
        .and_then(|size| tiny_skia::Pixmap::from_vec(rgba, size))
        .ok_or_else(|| format!("{width}x{height} is not a valid pixmap size"))?;
    pixmap
        .save_png(out)
        .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    Ok(format!("OK {width} {height} depth={}", image.depth))
}
