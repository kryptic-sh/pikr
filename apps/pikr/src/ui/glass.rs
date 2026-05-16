//! Glass overlay views: blurred backdrop + top glow.
//!
//! `backdrop_overlay` renders a captured-and-blurred screenshot behind the
//! panel content. `glow_overlay` paints a soft white-to-transparent gradient
//! along the top edge to simulate light hitting glass from above.

use floem::{
    IntoView,
    peniko::{Brush, Color, ColorStop, Gradient},
    views::{Decorators, container, img},
};

/// Render the blurred backdrop PNG as a full-size image view.
///
/// The `png_bytes` are the output of `crate::backdrop::capture_blurred`.
/// The image is stretched to fill the container — since it was cropped to
/// the panel dimensions at capture time, no distortion occurs.
pub fn backdrop_overlay(png_bytes: Vec<u8>) -> impl IntoView {
    img(move || png_bytes.clone()).style(|s| s.width_full().height_full())
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
