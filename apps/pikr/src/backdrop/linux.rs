//! Linux / FreeBSD backdrop capture via `grim`.
//!
//! `grim` uses the wlr-screencopy Wayland protocol. It ships with most
//! wlroots-based compositors (Sway, Hyprland, river, …). On X11 sessions
//! `grim` is unavailable; the function returns `None` gracefully so pikr
//! degrades to a flat panel without crashing.

use std::io::Cursor;
use std::process::{Command, Stdio};

use image::{DynamicImage, ImageFormat, imageops::FilterType};

/// Capture the screen via `grim`, blur it heavily, crop to `target_w × target_h`,
/// and return the result as PNG bytes.
///
/// Returns `None` on any failure — grim missing, non-zero exit, decode error,
/// encode error. The caller must handle `None` gracefully.
pub fn capture(target_w: u32, target_h: u32, sigma: f32) -> Option<Vec<u8>> {
    // ── 1. Shell out to grim ─────────────────────────────────────────────────
    let out = Command::new("grim")
        .args(["-t", "png", "-"])
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }

    // ── 2. Decode PNG → RGBA8 ────────────────────────────────────────────────
    let img = image::load_from_memory(&out.stdout).ok()?.into_rgba8();
    let (screen_w, screen_h) = (img.width(), img.height());

    // ── 3. Downsample → blur → upsample (much cheaper than full-res blur).
    //      Effective full-res sigma ≈ SIGMA × DOWNSCALE, so dialling either
    //      knob deepens the blur. Heavy downsample + moderate sigma is
    //      cheapest for the same visual strength.
    const DOWNSCALE: u32 = 16;
    let small_w = (screen_w / DOWNSCALE).max(1);
    let small_h = (screen_h / DOWNSCALE).max(1);

    let small = image::imageops::resize(&img, small_w, small_h, FilterType::Triangle);
    let blurred = image::imageops::blur(&small, sigma);
    // Second pass at full small-image scale for a creamier kernel — `blur()`
    // is a separable Gaussian, so two passes ≈ √2 × the effective sigma.
    let blurred = image::imageops::blur(&blurred, sigma);
    let sharp = image::imageops::resize(&blurred, screen_w, screen_h, FilterType::CatmullRom);

    // ── 4. Crop the centre slice sized to the picker panel ───────────────────
    // The wlr-layer-shell compositor always centers the surface on the output,
    // so the centre crop is a very good approximation. The blur is strong
    // enough that a few-pixel misalignment is invisible.
    let crop_w = target_w.min(screen_w);
    let crop_h = target_h.min(screen_h);

    let x = screen_w.saturating_sub(crop_w) / 2;
    let y = screen_h.saturating_sub(crop_h) / 2;

    let cropped = image::imageops::crop_imm(&sharp, x, y, crop_w, crop_h).to_image();

    // ── 5. Encode to PNG bytes ───────────────────────────────────────────────
    let dyn_img = DynamicImage::ImageRgba8(cropped);
    let mut buf = Vec::new();
    dyn_img
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .ok()?;

    Some(buf)
}
