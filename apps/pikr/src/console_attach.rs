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
