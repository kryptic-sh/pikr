//! Win32 icon resolution for drun targets.
//!
//! Extracts the icon for a given `.exe` / target path via `SHGetFileInfoW`,
//! converts the `HICON` to RGBA pixel data via `GetDIBits`, encodes the
//! result as a PNG, and caches it at
//! `%LOCALAPPDATA%\pikr\icon-cache\<sha256(target)>.png`.
//!
//! Subsequent runs skip the `SHGetFileInfoW` round-trip entirely and
//! return the cached path.  Cache invalidation is keyed solely by the
//! target path string — uninstall + reinstall of the same exe at the same
//! path keeps the stale icon; users can `rm -rf` the cache directory to
//! force regeneration.
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

/// Return the path to a cached PNG icon for `target`.
///
/// Returns `Some(path)` on success (cache hit or freshly written),
/// `None` if icon extraction fails (broken exe, GDI error, …).
pub fn icon_for(target: &Path) -> Option<PathBuf> {
    let cache_path = icon_cache_path(target)?;
    if cache_path.exists() {
        return Some(cache_path);
    }
    let bytes = extract_icon_png(target)?;
    std::fs::write(&cache_path, &bytes).ok()?;
    Some(cache_path)
}

/// Derive the on-disk path for `target`'s cached PNG.
///
/// The filename is the lower-hex SHA-256 of the target's UTF-8 string
/// representation, which keeps filenames short, filesystem-safe, and
/// deterministic.
fn icon_cache_path(target: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(target.to_string_lossy().as_bytes());
    let hash_hex = format!("{hash:x}");
    let cache_dir = dirs::data_local_dir()?.join("pikr").join("icon-cache");
    std::fs::create_dir_all(&cache_dir).ok()?;
    Some(cache_dir.join(format!("{hash_hex}.png")))
}

/// Extract the large icon for `target` via Win32 and encode it as PNG bytes.
///
/// Steps:
/// 1. `SHGetFileInfoW` — fills an `SHFILEINFOW` with the shell-assigned `HICON`.
/// 2. `GetIconInfo` — retrieves the colour bitmap handle (`hbmColor`).
/// 3. `GetObjectW` — reads width / height from the `BITMAP` struct.
/// 4. `GetDIBits` — copies pixel data into a `Vec<u8>` as 32 bpp BGRA,
///    top-down (negative `biHeight` forces top-down scan order).
/// 5. Swap blue ↔ red channels in-place to produce RGBA.
/// 6. Encode with `image::RgbaImage` → PNG bytes.
/// 7. `DestroyIcon` + `DeleteObject` for GDI handle cleanup.
///
/// Returns `None` on any step failure so the caller degrades gracefully.
fn extract_icon_png(target: &Path) -> Option<Vec<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, ReleaseDC,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    // Convert target path to a null-terminated UTF-16 string.
    let wide: Vec<u16> = OsStr::new(target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // --- Step 1: SHGetFileInfoW ---
    // dwFileAttributes is only consulted when SHGFI_USEFILEATTRIBUTES is set;
    // we leave it zeroed (no special file attributes passed).
    let mut shfi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let result = unsafe {
        SHGetFileInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result == 0 {
        return None;
    }

    let hicon = shfi.hIcon;
    if hicon.is_invalid() {
        return None;
    }

    // --- Step 2: GetIconInfo ---
    let mut icon_info: ICONINFO = unsafe { std::mem::zeroed() };
    if unsafe { GetIconInfo(hicon, &mut icon_info) }.is_err() {
        unsafe {
            let _ = DestroyIcon(hicon);
        }
        return None;
    }

    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    // Helper closure: clean up all GDI handles before returning.
    let cleanup = || unsafe {
        let _ = DestroyIcon(hicon);
        // HBITMAP implements CanInto<HGDIOBJ> so it satisfies DeleteObject's
        // P0: Param<HGDIOBJ> bound directly.
        if !hbm_color.is_invalid() {
            let _ = DeleteObject(hbm_color);
        }
        if !hbm_mask.is_invalid() {
            let _ = DeleteObject(hbm_mask);
        }
    };

    // --- Step 3: GetObjectW to read bitmap dimensions ---
    let mut bm: BITMAP = unsafe { std::mem::zeroed() };
    let got = unsafe {
        GetObjectW(
            hbm_color,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut core::ffi::c_void),
        )
    };
    if got == 0 {
        cleanup();
        return None;
    }

    let width = bm.bmWidth as u32;
    let height = bm.bmHeight as u32;
    if width == 0 || height == 0 {
        cleanup();
        return None;
    }

    // --- Step 4: GetDIBits — fills pixels as 32 bpp BGRA, top-down ---
    //
    // Negative biHeight forces top-down scan order (row 0 = top of image),
    // which matches `image::RgbaImage::from_raw` expectations.
    let stride = width * 4;
    let mut pixels: Vec<u8> = vec![0u8; (stride * height) as usize];

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // negative → top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..unsafe { std::mem::zeroed() }
    };

    // GetDC(HWND::default()) returns a screen DC suitable for GetDIBits
    // without a window association.
    let hdc = unsafe { GetDC(HWND::default()) };
    let rows_copied = unsafe {
        GetDIBits(
            hdc,
            hbm_color,
            0,
            height,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    unsafe { ReleaseDC(HWND::default(), hdc) };

    cleanup();

    if rows_copied == 0 {
        return None;
    }

    // --- Step 5: BGRA → RGBA ---
    // `GetDIBits` returns pixels in BGRA order (Windows GDI convention);
    // `image::RgbaImage` expects RGBA.  Swap B ↔ R channels in-place.
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2); // B ↔ R
    }

    // --- Step 6: Encode as PNG ---
    use image::ImageFormat;
    let img = image::RgbaImage::from_raw(width, height, pixels)?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache path for a given input string must be stable across calls.
    #[test]
    fn icon_cache_path_is_stable() {
        let target = Path::new(r"C:\Windows\System32\notepad.exe");
        let p1 = icon_cache_path(target);
        let p2 = icon_cache_path(target);
        assert_eq!(
            p1, p2,
            "cache path must be deterministic for the same input"
        );
    }

    /// Different targets must produce different cache paths.
    #[test]
    fn icon_cache_path_differs_for_different_targets() {
        let a = icon_cache_path(Path::new(r"C:\Windows\System32\notepad.exe"));
        let b = icon_cache_path(Path::new(r"C:\Windows\System32\calc.exe"));
        assert_ne!(a, b, "different targets must hash to different cache paths");
    }

    /// Extract the icon for notepad.exe and verify the result is a valid PNG.
    ///
    /// Skipped when notepad.exe is absent (sandboxed CI runners).
    #[test]
    fn icon_for_notepad_produces_png() {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if !notepad.exists() {
            return; // sandboxed CI — skip gracefully
        }
        let path = icon_for(notepad).expect("icon_for notepad.exe must succeed");
        assert!(path.exists(), "cached PNG must exist on disk");
        let bytes = std::fs::read(&path).expect("must be able to read cached PNG");
        // PNG magic bytes: \x89PNG\r\n\x1a\n
        assert_eq!(
            &bytes[..8],
            b"\x89PNG\r\n\x1a\n",
            "file must start with PNG magic"
        );
        // Second call must hit the on-disk cache (same path returned).
        let path2 = icon_for(notepad).expect("second call must also succeed");
        assert_eq!(path, path2, "cache path must be stable across calls");
    }
}
