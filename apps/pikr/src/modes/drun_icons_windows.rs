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
//!
//! When icon extraction fails (broken exe, no embedded resource, odd path,
//! permission error) `icon_for` falls back to Windows' generic-app icon,
//! cached once at `%LOCALAPPDATA%\pikr\icon-cache\__fallback__.png`.
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

/// Return the path to a cached PNG icon for `target`.
///
/// Returns `Some(path)` on success (cache hit or freshly written).
/// When extraction fails, falls back to the Windows generic-app icon
/// (also cached on disk).  Returns `None` only if even the fallback
/// cannot be produced.
pub fn icon_for(target: &Path) -> Option<PathBuf> {
    let cache_path = icon_cache_path(target)?;
    if cache_path.exists() {
        return Some(cache_path);
    }
    if let Some(bytes) = extract_icon_png(target) {
        std::fs::write(&cache_path, &bytes).ok()?;
        return Some(cache_path);
    }
    // Extraction failed (no icon resource, odd path, permission).  Fall
    // back to the Windows generic-app icon so the picker row still has
    // a visible slot.
    fallback_icon_path()
}

/// Derive the on-disk path for `target`'s cached PNG.
///
/// The filename is the lower-hex SHA-256 of the target's UTF-8 string
/// representation, which keeps filenames short, filesystem-safe, and
/// deterministic.
fn icon_cache_path(target: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(target.to_string_lossy().as_bytes());
    let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let cache_dir = dirs::data_local_dir()?.join("pikr").join("icon-cache");
    std::fs::create_dir_all(&cache_dir).ok()?;
    Some(cache_dir.join(format!("{hash_hex}.png")))
}

/// Return the path to the cached generic-app fallback PNG.
///
/// Extracts and writes `__fallback__.png` on first call; subsequent calls
/// return the cached path immediately.  Regenerated whenever the file is
/// missing (e.g. after a manual cache wipe).
fn fallback_icon_path() -> Option<PathBuf> {
    let cache_dir = dirs::data_local_dir()?.join("pikr").join("icon-cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    let fallback = cache_dir.join("__fallback__.png");
    if fallback.exists() {
        return Some(fallback);
    }
    let bytes = extract_generic_app_icon_png()?;
    std::fs::write(&fallback, &bytes).ok()?;
    Some(fallback)
}

/// Ask the shell for the generic `.exe` icon via `SHGFI_USEFILEATTRIBUTES`.
///
/// `SHGFI_USEFILEATTRIBUTES` tells the shell to skip any filesystem lookup
/// and return the icon associated with the *file type* of the supplied path.
/// Using a synthesized `application.exe` filename with `FILE_ATTRIBUTE_NORMAL`
/// therefore yields the standard Windows application icon regardless of
/// whether the file exists, following the user's current icon theme.
fn extract_generic_app_icon_png() -> Option<Vec<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{
        SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    // Synthesized path — never touched on disk thanks to USEFILEATTRIBUTES.
    let wide: Vec<u16> = OsStr::new("application.exe")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut shfi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let result = unsafe {
        SHGetFileInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
        )
    };
    if result == 0 || shfi.hIcon.is_invalid() {
        return None;
    }

    let hicon = shfi.hIcon;
    let png = hicon_to_png(hicon);
    // SHGFI_USEFILEATTRIBUTES still returns a caller-owned HICON; destroy it.
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    png
}

/// Convert a caller-owned `HICON` to PNG bytes.
///
/// Steps:
/// 1. `GetIconInfo`  — retrieves `hbmColor` / `hbmMask`.
/// 2. `GetObjectW`   — reads width / height from the `BITMAP` struct.
/// 3. `GetDIBits`    — copies pixel data as 32 bpp BGRA, top-down.
/// 4. BGRA → RGBA swap in-place.
/// 5. Encode with `image::RgbaImage` → PNG bytes.
/// 6. `DeleteObject` the bitmaps.
///
/// Does **not** call `DestroyIcon` on `hicon` — ownership stays with the
/// caller so both `extract_icon_png` and `extract_generic_app_icon_png` can
/// destroy it in their own cleanup paths.
///
/// Returns `None` on any step failure.
fn hicon_to_png(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, ReleaseDC,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    // --- Step 1: GetIconInfo ---
    let mut icon_info: ICONINFO = unsafe { std::mem::zeroed() };
    if unsafe { GetIconInfo(hicon, &mut icon_info) }.is_err() {
        return None;
    }

    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    // Helper closure: clean up bitmap handles before returning.
    let cleanup = || unsafe {
        if !hbm_color.is_invalid() {
            let _ = DeleteObject(hbm_color.into());
        }
        if !hbm_mask.is_invalid() {
            let _ = DeleteObject(hbm_mask.into());
        }
    };

    // --- Step 2: GetObjectW to read bitmap dimensions ---
    let mut bm: BITMAP = unsafe { std::mem::zeroed() };
    let got = unsafe {
        GetObjectW(
            hbm_color.into(),
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

    // --- Step 3: GetDIBits — fills pixels as 32 bpp BGRA, top-down ---
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
    let hdc = unsafe { GetDC(Some(HWND::default())) };
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
    unsafe { ReleaseDC(Some(HWND::default()), hdc) };

    cleanup();

    if rows_copied == 0 {
        return None;
    }

    // --- Step 4: BGRA → RGBA ---
    // `GetDIBits` returns pixels in BGRA order (Windows GDI convention);
    // `image::RgbaImage` expects RGBA.  Swap B ↔ R channels in-place.
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2); // B ↔ R
    }

    // --- Step 5: Encode as PNG ---
    use image::ImageFormat;
    let img = image::RgbaImage::from_raw(width, height, pixels)?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

/// Extract the large icon for `target` via Win32 and encode it as PNG bytes.
///
/// Steps:
/// 1. `SHGetFileInfoW` — fills an `SHFILEINFOW` with the shell-assigned `HICON`.
/// 2. Delegates GDI conversion to `hicon_to_png`.
/// 3. `DestroyIcon` for the caller-owned handle.
///
/// Returns `None` on any step failure so the caller degrades gracefully.
fn extract_icon_png(target: &Path) -> Option<Vec<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

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

    // --- Step 2: delegate GDI conversion ---
    let png = hicon_to_png(hicon);
    // SHGetFileInfoW returns a caller-owned HICON; destroy it regardless of
    // whether conversion succeeded.
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    png
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

    /// `fallback_icon_path` must return a path and the file must be a valid
    /// PNG once written.
    ///
    /// Skipped gracefully when `dirs::data_local_dir()` returns `None`
    /// (odd sandbox / non-Windows builds running the test via cfg(test)).
    #[cfg(all(test, windows))]
    #[test]
    fn fallback_icon_path_returns_a_path() {
        let Some(path) = fallback_icon_path() else {
            return; // no local-app-data dir — skip
        };
        assert!(
            path.exists(),
            "fallback PNG must exist on disk after first call"
        );
        let bytes = std::fs::read(&path).expect("must be able to read fallback PNG");
        assert_eq!(
            &bytes[..8],
            b"\x89PNG\r\n\x1a\n",
            "fallback file must start with PNG magic"
        );
        // Second call must hit the on-disk cache (file already present).
        let path2 = fallback_icon_path().expect("second call must also return Some");
        assert_eq!(path, path2, "fallback path must be stable across calls");
    }

    /// When given a nonexistent / iconless path, `icon_for` must return
    /// `Some` (the fallback) rather than `None`, so the picker row always
    /// has an icon slot filled.
    ///
    /// Skipped gracefully when `dirs::data_local_dir()` returns `None`.
    #[cfg(all(test, windows))]
    #[test]
    fn icon_for_unknown_path_falls_back() {
        // A path that cannot have an embedded icon resource.
        let path = Path::new(r"C:\nonexistent\thing.bat");
        let result = icon_for(path);
        // If data_local_dir is unavailable the fallback itself returns None;
        // that is the only acceptable None here.
        if dirs::data_local_dir().is_none() {
            return;
        }
        assert!(
            result.is_some(),
            "icon_for an unknown/iconless path must return Some (fallback)"
        );
    }
}
