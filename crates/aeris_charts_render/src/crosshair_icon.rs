//! Original browser SVG coverage, shared verbatim by every rendering backend.
//! Regenerate with `node examples/web_demo/build_crosshair_icon.mjs`.

use crate::draw_list::RasterImage;

pub const MAX_ICON_SIZE: u32 = 96;
const MASKS: &[u8] = include_bytes!("crosshair_add.alpha");

/// Decode one bounded mask. The engine retains only its current size, so steady frames
/// neither decode nor allocate. High-bit image keys are reserved for this built-in asset.
pub fn crosshair_icon(size: u32) -> RasterImage {
    let size = size.clamp(1, MAX_ICON_SIZE);
    let offset = |index: u32| {
        let start = index as usize * 4;
        u32::from_le_bytes(
            MASKS[start..start + 4]
                .try_into()
                .expect("built-in mask offset"),
        ) as usize
    };
    let mut pixels = Vec::with_capacity(size as usize * size as usize * 4);
    let (runs, remainder) = MASKS[offset(size - 1)..offset(size)].as_chunks::<2>();
    assert!(remainder.is_empty(), "built-in mask runs");
    for run in runs {
        for _ in 0..run[0] {
            pixels.extend_from_slice(&[255, 255, 255, run[1]]);
        }
    }
    assert_eq!(
        pixels.len(),
        size as usize * size as usize * 4,
        "built-in mask dimensions"
    );
    RasterImage {
        key: (1 << 63) | u64::from(size),
        width: size,
        height: size,
        pixels: pixels.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mask_decodes_to_its_declared_size() {
        for size in 1..=MAX_ICON_SIZE {
            let image = crosshair_icon(size);
            assert_eq!(image.pixels.len(), (size * size * 4) as usize);
            assert!(
                image
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] > 0)
            );
        }
        assert_eq!(crosshair_icon(0).width, 1);
        assert_eq!(crosshair_icon(u32::MAX).width, MAX_ICON_SIZE);
    }
}
