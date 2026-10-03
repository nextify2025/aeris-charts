//! WebGPU raster-image path. Immutable engine RGBA8 payloads upload once into a dedicated atlas;
//! retained frame groups then reuse the slot without competing with browser-rasterized text.

use aeris_charts_render::draw_list::Prim;
use aeris_charts_render_wgpu::{LabelAtlas, TexQuadInstance, ATLAS_SIZE};
use std::borrow::Cow;

fn premultiplied_pixels(pixels: &[u8]) -> Cow<'_, [u8]> {
    if pixels.as_chunks::<4>().0.iter().all(|rgba| rgba[3] == 255) {
        Cow::Borrowed(pixels)
    } else {
        let mut converted = pixels.to_vec();
        aeris_charts_render::draw_list::premultiply_rgba8(&mut converted);
        Cow::Owned(converted)
    }
}

pub(super) fn resolve(
    atlas: &mut LabelAtlas,
    queue: &wgpu::Queue,
    prim: &Prim,
) -> Option<TexQuadInstance> {
    let Prim::Image {
        image,
        rect,
        opacity,
    } = prim
    else {
        return None;
    };
    let rect = aeris_charts_render::draw_list::snap_image_rect(*rect)?;
    if image.width == 0
        || image.height == 0
        || image.width > ATLAS_SIZE
        || image.height > ATLAS_SIZE
        || image.pixels.len() != (image.width * image.height * 4) as usize
        || *opacity <= 0.0
    {
        return None;
    }
    let key = image.key.to_string();
    let slot = match atlas.get(&key) {
        Some(slot) => slot,
        None => {
            // Bilinear interpolation must operate on premultiplied colors, including the
            // transparent heatmap cells. This copy happens only on an image-atlas miss.
            let pixels = premultiplied_pixels(image.pixels.as_ref());
            atlas.insert(queue, key, image.width, image.height, pixels.as_ref())?
        }
    };
    Some(TexQuadInstance {
        rect,
        uv: slot.uv(),
        color: [1.0, 1.0, 1.0, opacity.clamp(0.0, 1.0)],
    })
}
