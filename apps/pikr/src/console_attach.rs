//! Windows-only `AttachConsole(ATTACH_PARENT_PROCESS)` shim.
//!
//! Pikr's release build links as `windows_subsystem = "windows"` so the Scoop
//! shim / Start Menu / Win+R launches don't flash a cmd window first. Side
//! effect: stdout / stderr are detached at process start, so any CLI path
//! that prints — `--version`, `--help`, `--dmenu` selection emit, panic
//! messages — silently drops its output. Worse, the caller never sees an
//! exit code from a windowed-subsystem process if it doesn't wait for it.
//!
//! Calling `AttachConsole(ATTACH_PARENT_PROCESS)` re-attaches the running
//! process to the parent shell's console (when one exists, i.e. when pikr
//! was launched from cmd / pwsh / a CI script). Rust's `println!` / `eprintln!`
//! then write to that console. When pikr is launched from Explorer / Scoop
//! shim, AttachConsole fails with `ERROR_INVALID_HANDLE` and the GUI path
//! proceeds unchanged — exactly what we want.

#![allow(unsafe_code)]

#[cfg(windows)]
pub fn attach_parent_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // Ignore the result: failure means no parent console (GUI launch),
    // which is the normal Scoop / Explorer path.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
#[inline]
pub fn attach_parent_console() {}

/// Runs `f` with no console and no standard handles, then re-attaches to
/// the parent's console if pikr had one, so later output still reaches it.
///
/// For launching a program that must get a console of its own: a child of a
/// process without std handles inherits none, and a console program given
/// none opens its own console with working ones. Measured on Windows 11 with
/// `pwsh -NoExit` and `cmd /K`: given NUL handles, or pikr's attached
/// console's handles, they closed or would share the launching terminal;
/// spawned in here they stayed open.
#[cfg(windows)]
pub fn without_console<T>(f: impl FnOnce() -> T) -> T {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{
        FreeConsole, GetConsoleWindow, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    // SAFETY: these calls take no pointers; they change only which console
    // and std handles this process has, which nothing else in pikr holds
    // across this call (the UI is gone by the time payloads run).
    let had_console = !unsafe { GetConsoleWindow() }.0.is_null();
    if had_console {
        unsafe {
            let _ = FreeConsole();
            for handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                let _ = SetStdHandle(handle, HANDLE::default());
            }
        }
    }
    let result = f();
    if had_console {
        attach_parent_console();
    }
    result
}
