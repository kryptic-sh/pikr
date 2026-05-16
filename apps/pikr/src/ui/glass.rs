//! Fake-glass overlay views: procedural noise grain + top glow.
//!
//! These are layered *on top* of existing content at very low alpha so text
//! legibility is unaffected. The grain reads as etched-glass texture; the glow
//! simulates light hitting glass from above.

use std::io::Cursor;

use floem::{
    IntoView,
    peniko::{Brush, Color, ColorStop, Gradient},
    views::{Decorators, container, img},
};
use image::{DynamicImage, ImageFormat, RgbaImage};

/// Side length of the noise tile in pixels. 128×128 is enough for the grain
/// to look stochastic; tiling artefacts are invisible at the low alpha we use.
const TILE_SIZE: u32 = 128;

/// Maximum per-channel alpha for the grain. Keeps noise subtle — etched
/// texture, not visual chaos.
const NOISE_MAX_ALPHA: u8 = 20;

/// Generate a 128×128 RGBA noise tile as a PNG-encoded `Vec<u8>`.
///
/// Uses an inline LCG so there is no dependency on the `rand` crate.
/// `seed` lets callers vary the pattern (e.g. for future animation ticks).
pub fn noise_png(seed: u32) -> Vec<u8> {
    let mut state: u32 = seed.wrapping_add(1); // avoid seed=0 fixed point
    let mut buf = RgbaImage::new(TILE_SIZE, TILE_SIZE);
    for pixel in buf.pixels_mut() {
        // LCG: glibc parameters
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let grey = ((state >> 16) & 0xff) as u8;
        // Alpha is proportional to grey intensity so darker grains are more
        // transparent, giving a natural-looking etched appearance.
        let alpha = (grey as u16 * NOISE_MAX_ALPHA as u16 / 255) as u8;
        *pixel = image::Rgba([grey, grey, grey, alpha]);
    }
    let dyn_img = DynamicImage::ImageRgba8(buf);
    let mut out = Vec::new();
    dyn_img
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .expect("noise_png: encode failed");
    out
}

/// A cached noise tile PNG. Call once at startup.
pub fn make_noise_bytes() -> Vec<u8> {
    noise_png(0xDEAD_BEEF)
}

/// The noise overlay: a single 128×128 tile stretched to fill its container.
///
/// Alpha is baked into the tile pixels (0..NOISE_MAX_ALPHA) so no extra
/// multiply is needed.
///
/// TODO: scroll — translate the tile slowly downward each frame to animate the
/// grain. Deferred until floem animation primitives are clearer.
pub fn noise_overlay(noise_bytes: Vec<u8>) -> impl IntoView {
    img(move || noise_bytes.clone()).style(|s| s.width_full().height_full())
}

/// The top-glow overlay: a vertical linear gradient, white at y=0 fading to
/// transparent at y=33% of the container height.
///
/// Because floem's `background()` gradient coordinates are relative to the
/// view's own bounding box, we place this in a container that covers only the
/// top third — the gradient then goes 0%→100% within that container.
pub fn glow_overlay(panel_height: f64) -> impl IntoView {
    let glow_h = panel_height / 3.0;
    let grad = Gradient::new_linear((0.0, 0.0), (0.0, glow_h)).with_stops([
        ColorStop::from((0.0_f32, Color::WHITE.multiply_alpha(0.12))),
        ColorStop::from((1.0_f32, Color::TRANSPARENT)),
    ]);
    container(floem::views::empty()).style(move |s| {
        s.absolute()
            .margin_top(0.0)
            .width_full()
            .height(glow_h)
            .background(Brush::Gradient(grad.clone()))
    })
}
