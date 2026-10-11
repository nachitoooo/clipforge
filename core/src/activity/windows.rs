//! The application in front, from the foreground window's process.
//!
//! Never run on a real Windows machine yet (F-2.4 is open); it is compiled and
//! linted by the CI cross-check. Anything that goes wrong — a game running as
//! administrator refuses `OpenProcess`, a program without version info — ends
//! as `None` or as the exe name, never as a failure.

use std::ffi::c_void;
use std::ptr;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::{AppInfo, ForegroundSource};

pub struct WindowsForeground;

impl ForegroundSource for WindowsForeground {
    fn foreground(&mut self) -> Option<AppInfo> {
        // SAFETY: plain Win32 calls; every pointer passed points at a local
        // that outlives the call.
        let path = unsafe {
            let window = GetForegroundWindow();
            if window.0.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid == 0 {
                return None;
            }
            image_path(pid)?
        };
        let file = file_name(&path);
        // SAFETY: as above.
        let described = unsafe { file_description(&path) };
        Some(AppInfo {
            id: file.to_lowercase(),
            name: described.unwrap_or_else(|| display_name(file)),
        })
    }
}

/// `C:\Games\Foo\Foo.exe` → `Foo.exe`.
fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// `Foo.exe` → `Foo`.
fn display_name(file: &str) -> String {
    let lower = file.to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(_) => file[..file.len() - 4].to_string(),
        None => file.to_string(),
    }
}

unsafe fn image_path(pid: u32) -> Option<String> {
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let queried = QueryFullProcessImageNameW(
        process,
        PROCESS_NAME_WIN32,
        PWSTR(buffer.as_mut_ptr()),
        &mut len,
    );
    CloseHandle(process).ok();
    queried.ok()?;
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

/// The "file description" in the exe's version resource, which is how most
/// games name themselves ("VALORANT"). `None` if it has none.
unsafe fn file_description(path: &str) -> Option<String> {
    let file = HSTRING::from(path);
    let size = GetFileVersionInfoSizeW(&file, None);
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    GetFileVersionInfoW(&file, None, size, data.as_mut_ptr().cast()).ok()?;

    // The block is keyed by language and code page; ask which ones it has.
    let (language, code_page) = {
        let mut found: *mut c_void = ptr::null_mut();
        let mut len = 0u32;
        let key = HSTRING::from("\\VarFileInfo\\Translation");
        if !VerQueryValueW(data.as_ptr().cast(), &key, &mut found, &mut len).as_bool()
            || found.is_null()
            || len < 4
        {
            return None;
        }
        let pair = std::slice::from_raw_parts(found as *const u16, 2);
        (pair[0], pair[1])
    };

    let key = HSTRING::from(format!(
        "\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"
    ));
    let mut text: *mut c_void = ptr::null_mut();
    let mut len = 0u32;
    if !VerQueryValueW(data.as_ptr().cast(), &key, &mut text, &mut len).as_bool()
        || text.is_null()
        || len == 0
    {
        return None;
    }
    let units = std::slice::from_raw_parts(text as *const u16, len as usize);
    let described = String::from_utf16_lossy(units);
    let described = described.trim_end_matches('\0').trim();
    (!described.is_empty()).then(|| described.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_name_is_the_last_part_of_the_path() {
        assert_eq!(file_name(r"C:\Games\Foo\Foo.exe"), "Foo.exe");
        assert_eq!(file_name("Foo.exe"), "Foo.exe");
        assert_eq!(file_name("C:/Games/Foo.exe"), "Foo.exe");
    }

    #[test]
    fn the_display_name_drops_the_exe_extension() {
        assert_eq!(
            display_name("VALORANT-Win64-Shipping.exe"),
            "VALORANT-Win64-Shipping"
        );
        assert_eq!(display_name("Game.EXE"), "Game");
        assert_eq!(display_name("tool"), "tool");
    }
}
