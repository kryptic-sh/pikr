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
        write_icon(&cache_path, &bytes).ok()?;
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

/// Derive the on-disk path for `key`'s cached PNG: where [`icon_for_app`]
/// writes the icon of the app whose id is `key`, whether or not it has yet.
///
/// The filename is the lower-hex SHA-256 of `key`, which keeps filenames
/// short, filesystem-safe, and deterministic — app ids contain `\`, `!` and
/// `://`.
pub fn icon_cache_path(key: &str) -> Option<PathBuf> {
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
    write_icon(&fallback, &bytes).ok()?;
    Some(fallback)
}

/// Write an icon's PNG `bytes` to `path` through `<path>.tmp`, renamed into
/// place once complete.
///
/// Unlike [`write_atomically`]'s, the temporary name is fixed: icons are
/// extracted in the background and pikr exits without waiting, so a write
/// can be cut off, and the next write of the same icon then replaces the
/// `.tmp` it left instead of adding another. The file is opened sharing only
/// deletion, so a second pikr writing the same icon fails to open it rather
/// than mixing its bytes in, and it is renamed while still open, so nobody
/// can reopen it between the last byte and the rename.
fn write_icon(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let tmp = icon_temp_path(path);
    let mut file = open_icon_temp(&tmp)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = written {
        // The error being returned is the one worth reporting; the `.tmp`
        // is still open to this handle alone, and removing it only tidies.
        let _ = std::fs::remove_file(&tmp);
        // Losing the race to another writer of the same icon is fine, as in
        // `write_atomically`.
        return if path.exists() { Ok(()) } else { Err(e) };
    }
    Ok(())
}

/// `<path>.tmp`, where [`write_icon`] writes `path` before renaming it.
fn icon_temp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// Opens `tmp` empty, for [`write_icon`] alone: shared only for deletion, so
/// no other writer can open it until this handle is closed.
fn open_icon_temp(tmp: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;

    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(FILE_SHARE_DELETE.0)
        .open(tmp)
}

/// Write `bytes` to a uniquely named sibling temp file, then rename it over
/// `path`.
///
/// Several pikr processes can write the drun app-list cache at once, and its
/// background refresh can be cut off by pikr exiting. [`crate::picker::write_private_state`] renames
/// a complete file into place, so readers only ever see the old file or the
/// new one.
pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    crate::picker::write_private_state(path, bytes).or_else(|e| {
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
/// 3. BGRA → RGBA through `crcbl_image::raw::convert`.
/// 4. Encode with `crcbl_image::png::encode` → PNG bytes.
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
    // which `RowOrder::TopDown` below declares.
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

    // --- Steps 3 and 4: BGRA → RGBA, then PNG ---
    // The alpha bytes are taken as they come, straight and never replaced,
    // as this path has always read them.
    use crcbl_image::raw::{AllZeroAlpha, Alpha, ChannelOrder, RawPixels, RowOrder};
    let raw = RawPixels {
        width,
        height,
        stride: width as usize * 4,
        order: ChannelOrder::Bgra,
        rows: RowOrder::TopDown,
        alpha: Alpha::Straight,
        all_zero_alpha: AllZeroAlpha::Transparent,
        data: &pixels,
    };
    let image = crcbl_image::raw::convert(&raw, &crcbl_image::Limits::DEFAULT).ok()?;
    crcbl_image::png::encode(&image).ok()
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

    /// Different keys must produce different cache paths.
    #[test]
    fn icon_cache_path_differs_for_different_keys() {
        let a = icon_cache_path("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App");
        let b = icon_cache_path("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App");
        assert_ne!(a, b, "different keys must hash to different cache paths");
    }

    /// A write cut off by pikr exiting leaves `<icon>.tmp`; the next write
    /// of that icon takes it over rather than adding a file beside it.
    #[test]
    fn write_icon_replaces_a_temporary_file_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.png");
        std::fs::write(dir.path().join("icon.png.tmp"), b"cut off").unwrap();
        write_icon(&path, b"whole").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"whole");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["icon.png"]);
    }

    /// While another writer has `<icon>.tmp` open, a second one is refused
    /// instead of writing into it.
    #[test]
    fn write_icon_is_refused_while_another_writer_holds_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.png");
        let _other = open_icon_temp(&icon_temp_path(&path)).unwrap();
        assert!(write_icon(&path, b"whole").is_err());
        assert!(!path.exists());
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
