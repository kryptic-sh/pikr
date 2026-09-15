//! Win32 icon resolution for drun entries.
//!
//! Asks each `shell:AppsFolder` item for its icon through
//! `IShellItemImageFactory` — the same source Start uses, so packaged apps and
//! shortcuts without an on-disk target get their real icon — converts the
//! bitmap to RGBA via `GetDIBits`, encodes it as PNG, and caches it at
//! `%LOCALAPPDATA%\pikr\icon-cache\<sha256(app id)>.png`.
//!
//! Subsequent runs skip extraction and return the cached path. Cache
//! invalidation is keyed solely by the app id — an app that changes its icon
//! keeps the stale one; deleting the cache directory forces regeneration.
//!
//! When extraction fails `icon_for_app` falls back to Windows' generic-app
//! icon, cached once at `%LOCALAPPDATA%\pikr\icon-cache\__fallback__.png`.
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::Shell::IShellItem;

/// Requested icon edge in pixels. Rows render icons at 24 px; a 32 px source
/// downscales cleanly (the same request size the unix theme lookup uses).
const ICON_PX: i32 = 32;

/// Return the path to a cached PNG icon for the AppsFolder `item` whose
/// parsing name is `app_id`.
///
/// Returns `Some(path)` on success (cache hit or freshly written). When
/// extraction fails, falls back to the Windows generic-app icon (also cached
/// on disk). Returns `None` only if even the fallback cannot be produced.
pub fn icon_for_app(item: &IShellItem, app_id: &str) -> Option<PathBuf> {
    let cache_path = icon_cache_path(app_id)?;
    if cache_path.exists() {
        return Some(cache_path);
    }
    if let Some(bytes) = extract_item_icon_png(item) {
        write_atomically(&cache_path, &bytes).ok()?;
        return Some(cache_path);
    }
    // Extraction failed. Fall back to the generic-app icon so the picker row
    // still has a visible slot.
    fallback_icon_path()
}

/// `%LOCALAPPDATA%\pikr\icon-cache`, created if missing.
fn cache_dir() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("pikr").join("icon-cache");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Derive the on-disk path for `key`'s cached PNG.
///
/// The filename is the lower-hex SHA-256 of `key`, which keeps filenames
/// short, filesystem-safe, and deterministic — app ids contain `\`, `!` and
/// `://`.
fn icon_cache_path(key: &str) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(key.as_bytes());
    let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    Some(cache_dir()?.join(format!("{hash_hex}.png")))
}

/// Return the path to the cached generic-app fallback PNG.
///
/// Extracts and writes `__fallback__.png` on first call; subsequent calls
/// return the cached path immediately. Regenerated whenever the file is
/// missing (e.g. after a manual cache wipe).
fn fallback_icon_path() -> Option<PathBuf> {
    let fallback = cache_dir()?.join("__fallback__.png");
    if fallback.exists() {
        return Some(fallback);
    }
    let bytes = extract_generic_app_icon_png()?;
    write_atomically(&fallback, &bytes).ok()?;
    Some(fallback)
}

/// Write `bytes` to a uniquely named sibling temp file, then rename it over
/// `path`.
///
/// Several entries can share one cache file (every miss shares
/// `__fallback__.png`), and several pikr processes can fill the caches at
/// once — including the drun app-list cache, whose background refresh can be
/// cut off by pikr exiting. A plain `fs::write` truncates in place, so a
/// concurrent writer, a reader that saw `exists()`, or the next launch after a
/// kill could observe a half-written file. Renaming a complete file means
/// readers only ever see the old file or the new one.
pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).or_else(|e| {
        let _ = std::fs::remove_file(&tmp);
        // Losing the race to another writer that produced the same file is
        // fine; Windows refuses to replace a file another handle has open.
        if path.exists() { Ok(()) } else { Err(e) }
    })
}

/// Render `item`'s icon (never a content thumbnail) to PNG bytes.
fn extract_item_icon_png(item: &IShellItem) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SIIGBF_ICONONLY};
    use windows::core::Interface;

    let factory: IShellItemImageFactory = item.cast().ok()?;
    let size = SIZE {
        cx: ICON_PX,
        cy: ICON_PX,
    };
    let hbm = unsafe { factory.GetImage(size, SIIGBF_ICONONLY) }.ok()?;
    let png = hbitmap_to_png(hbm);
    // GetImage hands the caller ownership of the bitmap.
    unsafe {
        let _ = DeleteObject(hbm.into());
    }
    png
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

/// Convert a caller-owned `HICON` to PNG bytes via its colour bitmap.
///
/// Does **not** call `DestroyIcon` on `hicon` — ownership stays with the
/// caller. The colour and mask bitmaps `GetIconInfo` creates are deleted here.
fn hicon_to_png(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut icon_info: ICONINFO = unsafe { std::mem::zeroed() };
    unsafe { GetIconInfo(hicon, &mut icon_info) }.ok()?;

    let png = hbitmap_to_png(icon_info.hbmColor);
    unsafe {
        if !icon_info.hbmColor.is_invalid() {
            let _ = DeleteObject(icon_info.hbmColor.into());
        }
        if !icon_info.hbmMask.is_invalid() {
            let _ = DeleteObject(icon_info.hbmMask.into());
        }
    }
    png
}

/// Copy a bitmap's pixels out as 32 bpp and encode them as PNG bytes.
///
/// Steps:
/// 1. `GetObjectW` — reads width / height from the `BITMAP` struct.
/// 2. `GetDIBits`  — copies pixel data as 32 bpp BGRA, top-down.
/// 3. BGRA → RGBA swap in-place.
/// 4. Encode with `image::RgbaImage` → PNG bytes.
///
/// Leaves `hbm` alive; the caller owns it. Returns `None` on any step failure.
fn hbitmap_to_png(hbm: HBITMAP) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW,
        ReleaseDC,
    };

    if hbm.is_invalid() {
        return None;
    }

    // --- Step 1: GetObjectW to read bitmap dimensions ---
    let mut bm: BITMAP = unsafe { std::mem::zeroed() };
    let got = unsafe {
        GetObjectW(
            hbm.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut core::ffi::c_void),
        )
    };
    if got == 0 {
        return None;
    }

    // `GetDIBits` writes `width * height * 4` bytes into `pixels`, so the
    // buffer size must be computed without wrapping — a wrapped product
    // would under-allocate and let GDI write past the end.
    let (Ok(width), Ok(height)) = (u32::try_from(bm.bmWidth), u32::try_from(bm.bmHeight)) else {
        return None;
    };
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|&n| n > 0)?;

    // --- Step 2: GetDIBits — fills pixels as 32 bpp BGRA, top-down ---
    //
    // Negative biHeight forces top-down scan order (row 0 = top of image),
    // which matches `image::RgbaImage::from_raw` expectations.
    let mut pixels: Vec<u8> = vec![0u8; len];

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bm.bmWidth,
            biHeight: -bm.bmHeight, // negative → top-down
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
            hbm,
            0,
            height,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    unsafe { ReleaseDC(Some(HWND::default()), hdc) };

    if rows_copied == 0 {
        return None;
    }

    // --- Step 3: BGRA → RGBA ---
    // `GetDIBits` returns pixels in BGRA order (Windows GDI convention);
    // `image::RgbaImage` expects RGBA.  Swap B ↔ R channels in-place.
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2); // B ↔ R
    }

    // --- Step 4: Encode as PNG ---
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

    /// The cache path for a given key must be stable across calls.
    #[test]
    fn icon_cache_path_is_stable() {
        let key = "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App";
        assert_eq!(
            icon_cache_path(key),
            icon_cache_path(key),
            "cache path must be deterministic for the same input"
        );
    }

    /// Concurrent writers of one cache file must never expose a partial file
    /// to a reader.
    #[test]
    fn write_atomically_never_exposes_partial_file() {
        const LEN: usize = 256 * 1024;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.png");
        write_atomically(&path, &vec![0u8; LEN]).unwrap();

        std::thread::scope(|s| {
            for w in 0..4u8 {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..25 {
                        write_atomically(path, &vec![w; LEN]).unwrap();
                    }
                });
            }
            for _ in 0..4 {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..100 {
                        // A read can lose to a rename in progress on Windows;
                        // only a successful read is checked for completeness.
                        if let Ok(bytes) = std::fs::read(path) {
                            assert_eq!(bytes.len(), LEN, "reader saw a partial file");
                        }
                    }
                });
            }
        });
    }

    /// Different keys must produce different cache paths.
    #[test]
    fn icon_cache_path_differs_for_different_keys() {
        let a = icon_cache_path("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App");
        let b = icon_cache_path("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App");
        assert_ne!(a, b, "different keys must hash to different cache paths");
    }

    /// `fallback_icon_path` must return a path and the file must be a valid
    /// PNG once written.
    ///
    /// Skipped gracefully when `dirs::data_local_dir()` returns `None`.
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
}
