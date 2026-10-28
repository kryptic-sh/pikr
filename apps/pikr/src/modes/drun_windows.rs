//! Windows drun — enumerates the shell's `AppsFolder`.
//!
//! `shell:AppsFolder` is the virtual folder behind Start's "All apps" list. It
//! merges the per-user and all-users Start Menu shortcuts, packaged (Store /
//! MSIX) apps, and protocol launchers such as `steam://` games into one list
//! of shell items. Each item's parent-relative parsing name is its app id:
//! an AppUserModelID (`Microsoft.WindowsNotepad_8wekyb3d8bbwe!App`), a
//! known-folder path (`{1AC14E77-…}\cmd.exe`), an absolute path, or a URL.
//!
//! Accepting an entry hands `shell:AppsFolder\<id>` to `ShellExecuteExW`
//! ([`launch`]), which activates it the way Start does, packaged apps and
//! MSI-advertised shortcuts included. Walking `.lnk` files directly cannot:
//! only shortcuts whose LinkInfo block names an on-disk target resolve, which
//! drops system tools (Command Prompt, Task Manager), MSI-advertised shortcuts
//! (LibreOffice), and every packaged app.
//!
//! The shell builds the merged list lazily while it is enumerated, which costs
//! well over a hundred milliseconds even when warm. So [`collect`] serves the
//! previous launch's list from `%LOCALAPPDATA%\pikr\drun-apps.toml` and
//! re-enumerates on a background thread, rewriting the cache only when the list
//! changed: an app installed or removed since shows up one launch later.
//!
//! Extracting the icons is far slower still, seconds for a full Start menu. So
//! with no cache, [`collect`] lists the apps without extracting, each pointing
//! at the file its icon will be cached in, and the background enumeration
//! extracts them. A row draws its icon once the file lands.
#![allow(unsafe_code)]

use super::icons_windows;
use crate::modes::{Entry, Payload};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
    SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

/// `shell:` namespace prefix that turns an app id back into its AppsFolder
/// item for [`launch`].
const APPS_FOLDER: &str = r"shell:AppsFolder\";

/// Target extensions of shortcuts that open a document rather than a program
/// (release notes, help pages, web links saved as `.url`).
const DOCUMENT_EXTS: &[&str] = &["txt", "url", "pdf", "html", "htm"];

/// One launchable AppsFolder item, as listed and as cached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct App {
    label: String,
    /// Parsing name relative to `shell:AppsFolder`; see the module docs.
    id: String,
    /// Absolute path of the cached PNG icon.
    icon: Option<String>,
}

/// On-disk form of the app list.
#[derive(Serialize, Deserialize)]
struct CachedApps {
    apps: Vec<App>,
}

/// How [`enumerate`] fills in each app's icon.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Icons {
    /// Extract any icon not cached yet, as the list is walked.
    Extract,
    /// Point at the file the icon is cached in, whether or not it is there
    /// yet; a later [`Icons::Extract`] walk writes it.
    Deferred,
}

/// The apps to list, and, when their icons are still being extracted, a
/// message for each icon file as it lands.
pub fn collect() -> Result<(Vec<Entry>, Option<Receiver<()>>)> {
    let _span = tracing::debug_span!("drun_collect_windows").entered();
    let Some(cache) = cache_path() else {
        return Ok((to_entries(enumerate(Icons::Extract, &|| {})?), None));
    };
    let collected = collect_cached(cache, enumerate)?;
    // Never waited for: pikr exits as soon as the user picks, and a refresh
    // cut off there leaves every icon it wrote in place for the next launch.
    drop(collected.refresh);
    Ok((to_entries(collected.shown), collected.landed))
}

/// What [`collect_cached`] lists now, and what it leaves running.
struct Collected {
    shown: Vec<App>,
    /// A message for each icon file the refresh puts in place, when the
    /// icons of `shown` were deferred.
    landed: Option<Receiver<()>>,
    /// The enumeration with icons extracted.
    refresh: JoinHandle<()>,
}

/// The list to show now, from `cache` or else from `enumerate` with its icons
/// deferred, and the thread that enumerates again with icons extracted and
/// rewrites `cache` when the list differs from the one shown.
fn collect_cached(
    cache: PathBuf,
    enumerate: impl Fn(Icons, &dyn Fn()) -> Result<Vec<App>> + Send + 'static,
) -> Result<Collected> {
    if let Some(apps) = load_cache(&cache) {
        let previous = apps.clone();
        let refresh =
            std::thread::spawn(move || refresh_cache(&cache, &previous, enumerate, &|| {}));
        return Ok(Collected {
            shown: apps,
            landed: None,
            refresh,
        });
    }
    let shown = enumerate(Icons::Deferred, &|| {})?;
    let (tx, landed) = mpsc::channel();
    let refresh = std::thread::spawn(move || {
        // Nothing was cached, so the refresh writes what it finds even when
        // it matches the list shown. A send fails only once the window has
        // dropped the receiver, when there is nothing left to draw.
        let landed = || {
            let _ = tx.send(());
        };
        refresh_cache(&cache, &[], enumerate, &landed);
    });
    Ok(Collected {
        shown,
        landed: Some(landed),
        refresh,
    })
}

/// Activate the AppsFolder item `app_id` through the shell, as Start does.
pub fn launch(app_id: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let _com = ComApartment::enter();
    let target: Vec<u16> = std::ffi::OsStr::new(&format!("{APPS_FOLDER}{app_id}"))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // pikr exits as soon as `execute` returns; NOASYNC makes the call
        // finish the activation before it does.
        fMask: SEE_MASK_NOASYNC,
        lpFile: windows::core::PCWSTR(target.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.with_context(|| format!("launch {app_id}"))
}

fn to_entries(apps: Vec<App>) -> Vec<Entry> {
    apps.into_iter()
        .map(|app| Entry {
            label: app.label,
            description: None,
            icon: app.icon,
            payload: Payload::ShellApp(app.id),
        })
        .collect()
}

/// Walk `shell:AppsFolder` and return its launchable items sorted by label,
/// with their icons as `icons` says, calling `landed` once each extracted
/// icon's file is in place.
fn enumerate(icons: Icons, landed: &dyn Fn()) -> Result<Vec<App>> {
    let _span = tracing::debug_span!("drun_apps_folder_enumerate").entered();
    let _com = ComApartment::enter();

    let folder: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) }
            .context("open shell:AppsFolder")?;
    let items: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems) }
        .context("enumerate shell:AppsFolder")?;

    let mut apps = Vec::new();
    loop {
        let mut slot = [None];
        // `Next` reports the end of the list as S_FALSE, which windows-rs
        // maps to `Ok` — an empty slot is the real end-of-list signal.
        unsafe { items.Next(&mut slot, None) }.context("read shell:AppsFolder item")?;
        let Some(item) = slot[0].take() else { break };
        let (Some(label), Some(id)) = (
            display_name(&item, SIGDN_NORMALDISPLAY),
            display_name(&item, SIGDN_PARENTRELATIVEPARSING),
        ) else {
            continue;
        };
        if !is_launchable(&label, &id) {
            continue;
        }
        let icon = match icons {
            Icons::Extract => icons_windows::icon_for_app(&item, &id).inspect(|_| landed()),
            Icons::Deferred => icons_windows::icon_cache_path(&id),
        };
        let icon = icon.map(|path| path.to_string_lossy().into_owned());
        apps.push(App { label, id, icon });
    }

    apps.sort_by_key(|app| app.label.to_lowercase());
    tracing::debug!(apps = apps.len(), "drun apps folder enumerated");
    Ok(apps)
}

/// Re-enumerate with icons extracted, calling `landed` for each, and rewrite
/// the cache if the list differs from `previous`. Runs on its own thread, so
/// it joins its own COM apartment inside `enumerate`.
fn refresh_cache(
    cache: &Path,
    previous: &[App],
    enumerate: impl Fn(Icons, &dyn Fn()) -> Result<Vec<App>>,
    landed: &dyn Fn(),
) {
    match enumerate(Icons::Extract, landed) {
        Ok(apps) if apps != previous => {
            if let Err(e) = write_cache(cache, &apps) {
                tracing::warn!("drun app cache write failed: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("drun app cache refresh failed: {e:#}"),
    }
}

/// `%LOCALAPPDATA%\pikr\drun-apps.toml`.
fn cache_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("pikr").join("drun-apps.toml"))
}

/// The cached list, or `None` when there is no cache or it does not parse.
fn load_cache(path: &Path) -> Option<Vec<App>> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<CachedApps>(&text)
        .map_err(|e| tracing::warn!("drun app cache parse error: {e}"))
        .ok()
        .map(|cached| cached.apps)
}

fn write_cache(path: &Path, apps: &[App]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(&CachedApps {
        apps: apps.to_vec(),
    })
    .map_err(std::io::Error::other)?;
    icons_windows::write_atomically(path, text.as_bytes())
}

/// Whether an AppsFolder item is a program to list, as opposed to a document,
/// web link, or uninstaller that Start also files under "All apps".
///
/// Custom-scheme ids (`steam://rungameid/…`) are kept — they launch games —
/// but plain `http(s)://` ids are web pages.
fn is_launchable(label: &str, app_id: &str) -> bool {
    if label.is_empty() || app_id.is_empty() {
        return false;
    }
    if label.to_lowercase().contains("uninstall") {
        return false;
    }
    let id = app_id.to_ascii_lowercase();
    if id.starts_with("http://") || id.starts_with("https://") {
        return false;
    }
    let is_document = Path::new(app_id)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| DOCUMENT_EXTS.iter().any(|d| ext.eq_ignore_ascii_case(d)));
    !is_document
}

/// Read one of `item`'s names, freeing the shell-allocated buffer. `None` when
/// the shell has no such name or it is not valid UTF-16.
fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    let raw = unsafe { item.GetDisplayName(kind) }.ok()?;
    let name = unsafe { raw.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(raw.0.cast_const().cast())) };
    name
}

/// A COM apartment joined for the current thread for as long as the guard
/// lives. Declare it before any COM interface so those drop first.
///
/// Joins a single-threaded apartment, as `ShellExecuteExW`'s documentation
/// asks for: some shell extensions it may load need STA. If the thread is
/// already in a multithreaded apartment the join fails with
/// `RPC_E_CHANGED_MODE`; the shell calls here still work there, so the guard
/// just doesn't release it. Clipboard mode uses it too, on a thread of its
/// own, since WinRT's `Clipboard` refuses activation from an MTA.
pub(crate) struct ComApartment {
    joined: bool,
}

impl ComApartment {
    pub(crate) fn enter() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self { joined: hr.is_ok() }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.joined {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_launchable() {
        for (label, id) in [
            ("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
            (
                "Command Prompt",
                r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\cmd.exe",
            ),
            (
                "Riot Client",
                r"C:\Riot Games\Riot Client\RiotClientServices.exe",
            ),
            (
                "LibreOffice Writer",
                "TheDocumentFoundation.LibreOffice.Writer",
            ),
            ("A Game", "steam://rungameid/3244220"),
            (
                "7-Zip Help",
                r"{6D809377-6AF0-444B-8957-A3773F02200E}\7-Zip\7-zip.chm",
            ),
        ] {
            assert!(is_launchable(label, id), "{label} [{id}] must be listed");
        }
    }

    #[test]
    fn documents_links_and_uninstallers_are_skipped() {
        for (label, id) in [
            ("Git FAQs", "https://gitforwindows.org/faq"),
            ("Steam Support Center", "http://support.steampowered.com/"),
            (
                "Release Notes",
                r"{6D809377-6AF0-444B-8957-A3773F02200E}\VideoLAN\VLC\NEWS.txt",
            ),
            (
                "Documentation",
                r"{6D809377-6AF0-444B-8957-A3773F02200E}\VLC\Documentation.URL",
            ),
            (
                "Git Release Notes",
                r"{6D809377-6AF0-444B-8957-A3773F02200E}\Git\ReleaseNotes.html",
            ),
            ("Uninstall Foo", r"C:\Program Files\Foo\unins000.exe"),
            ("", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
            ("Notepad", ""),
        ] {
            assert!(!is_launchable(label, id), "{label} [{id}] must be skipped");
        }
    }

    fn app(label: &str, id: &str) -> App {
        App {
            label: label.into(),
            id: id.into(),
            icon: Some(format!(r"C:\cache\{label}.png")),
        }
    }

    #[test]
    fn cache_round_trips_apps_with_shell_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-apps.toml");
        let apps = vec![
            app(
                "Command Prompt",
                r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\cmd.exe",
            ),
            app("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
            App {
                icon: None,
                ..app("A Game", "steam://rungameid/3244220")
            },
        ];
        write_cache(&path, &apps).unwrap();
        assert_eq!(load_cache(&path), Some(apps));
    }

    #[test]
    fn unparseable_cache_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-apps.toml");
        // A truncated file, shaped like the old JSON cache.
        std::fs::write(&path, br#"{"roots_mtime_unix_nanos": 1, "entries": ["#).unwrap();
        assert_eq!(load_cache(&path), None);
        assert_eq!(load_cache(&dir.path().join("missing.toml")), None);
    }

    #[test]
    fn cached_apps_become_shell_app_entries() {
        let entries = to_entries(vec![app(
            "Notepad",
            "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
        )]);
        assert_eq!(entries[0].label, "Notepad");
        assert_eq!(entries[0].icon.as_deref(), Some(r"C:\cache\Notepad.png"));
        assert!(matches!(
            &entries[0].payload,
            Payload::ShellApp(id) if id == "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"
        ));
    }

    /// Stands in for the shell's list: deferred icons under `C:\deferred`,
    /// extracted ones under `C:\extracted`, each reported as it lands.
    fn two_apps(icons: Icons, landed: &dyn Fn()) -> Result<Vec<App>> {
        let dir = match icons {
            Icons::Deferred => "deferred",
            Icons::Extract => "extracted",
        };
        Ok(["Notepad", "Paint"]
            .map(|label| {
                if icons == Icons::Extract {
                    landed();
                }
                App {
                    label: label.into(),
                    id: label.into(),
                    icon: Some(format!(r"C:\{dir}\{label}.png")),
                }
            })
            .into())
    }

    #[test]
    fn a_cold_collect_shows_deferred_icons_and_the_refresh_extracts_and_caches_them() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("drun-apps.toml");
        let collected = collect_cached(cache.clone(), two_apps).unwrap();
        assert_eq!(collected.shown, two_apps(Icons::Deferred, &|| {}).unwrap());
        collected.refresh.join().unwrap();
        let landed = collected
            .landed
            .expect("a cold collect reports its icons landing");
        assert_eq!(landed.try_iter().count(), 2, "one message per icon");
        assert_eq!(
            load_cache(&cache),
            Some(two_apps(Icons::Extract, &|| {}).unwrap())
        );
    }

    #[test]
    fn a_cold_collect_caches_a_list_that_matches_the_one_shown() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("drun-apps.toml");
        let same = |_: Icons, _: &dyn Fn()| Ok(vec![app("Notepad", "notepad")]);
        let collected = collect_cached(cache.clone(), same).unwrap();
        collected.refresh.join().unwrap();
        assert_eq!(load_cache(&cache), Some(vec![app("Notepad", "notepad")]));
    }

    #[test]
    fn a_warm_collect_shows_the_cache_and_defers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("drun-apps.toml");
        write_cache(&cache, &[app("Notepad", "notepad")]).unwrap();
        let extract_only = |icons: Icons, landed: &dyn Fn()| {
            assert_eq!(icons, Icons::Extract, "a warm collect never defers");
            two_apps(icons, landed)
        };
        let collected = collect_cached(cache.clone(), extract_only).unwrap();
        assert_eq!(collected.shown, vec![app("Notepad", "notepad")]);
        assert!(collected.landed.is_none(), "nothing is on its way");
        collected.refresh.join().expect("the refresh extracts");
        assert_eq!(
            load_cache(&cache),
            Some(two_apps(Icons::Extract, &|| {}).unwrap()),
            "a changed list is cached"
        );
    }

    /// The enumeration must see what the `.lnk` walker could not: Command
    /// Prompt's shortcut stores its target as an IDList plus `%windir%`
    /// environment block, with no LinkInfo, so the walker dropped it.
    #[test]
    fn enumerate_lists_system_tools_with_icons() {
        let landed = std::cell::Cell::new(0);
        let apps = enumerate(Icons::Extract, &|| landed.set(landed.get() + 1))
            .expect("shell:AppsFolder must enumerate");
        assert_eq!(
            landed.get(),
            apps.iter().filter(|app| app.icon.is_some()).count(),
            "each icon in place is reported"
        );
        assert!(
            apps.iter()
                .any(|app| app.id.to_ascii_lowercase().ends_with(r"\cmd.exe")),
            "Command Prompt must be listed; got {} apps",
            apps.len()
        );
        let icon = apps
            .iter()
            .find_map(|app| app.icon.as_deref())
            .expect("apps must carry cached icons");
        let bytes = std::fs::read(icon).expect("cached icon must be readable");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "icon must be a PNG");
    }
}
