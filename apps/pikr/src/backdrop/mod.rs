//! Platform-agnostic desktop backdrop capture.
//!
//! Captures the full desktop, applies a heavy Gaussian blur, then crops a
//! centre slice sized to the picker panel. Returns PNG-encoded bytes ready for
//! a floem `img` view.
//!
//! Platform dispatch:
//! - Linux / FreeBSD: shell out to `grim` (wlr-screencopy). See `linux.rs`.
//! - macOS: stub — `None`. See `macos.rs`.
//! - Windows: stub — `None`. See `windows.rs`.
//! - other targets: `None` inline.
//!
//! The function is designed to be called **once at startup**, not on every
//! frame. If capture fails for any reason (grim not installed, permission
//! error, OOM) it returns `None` and the caller simply skips the overlay.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Capture the desktop, apply Gaussian blur at the given sigma, return PNG
/// bytes sized to the panel. Returns `None` if capture is unsupported or
/// fails — the caller falls back to a flat-color panel. `sigma` is applied
/// at the downsampled scale; effective full-res blur ≈ sigma × downscale.
///
/// When `grayscale` is true, each pixel is desaturated to its Rec. 709
/// luminance after blur — used by `--smoked` for a neutral backdrop.
pub fn capture_blurred(
    target_w: u32,
    target_h: u32,
    sigma: f32,
    grayscale: bool,
) -> Option<Vec<u8>> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    return linux::capture(target_w, target_h, sigma, grayscale);

    #[cfg(target_os = "macos")]
    return macos::capture(target_w, target_h, sigma, grayscale);

    #[cfg(target_os = "windows")]
    return windows::capture(target_w, target_h, sigma, grayscale);

    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "windows",
    )))]
    {
        let _ = (target_w, target_h, sigma, grayscale);
        None
    }
}
