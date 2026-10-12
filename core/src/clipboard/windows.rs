//! A `CF_HDROP` list on the clipboard: what Explorer's "Copy" puts there, so
//! pasting in a chat, a mail or a folder takes the file.
//!
//! Never run on a real Windows machine yet (F-2.4 is open); it is compiled and
//! linted by the CI cross-check. The clipboard can be held by another program
//! for a moment, so opening it is retried briefly.

use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_HDROP;

const OPEN_TRIES: u32 = 10;
const OPEN_WAIT: Duration = Duration::from_millis(20);

pub fn copy_file(path: &Path) -> Result<(), String> {
    let payload = super::hdrop_payload(path)?;

    // SAFETY: plain Win32 calls. `memory` is ours until `SetClipboardData`
    // succeeds (then the system owns it and it must not be freed); the copy
    // stays inside the `payload.len()` bytes that were allocated.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, payload.len())
            .map_err(|e| format!("no se pudo reservar memoria para el portapapeles: {e}"))?;
        if let Err(e) = fill(memory, &payload) {
            GlobalFree(Some(memory)).ok();
            return Err(e);
        }
        if let Err(e) = open_clipboard() {
            GlobalFree(Some(memory)).ok();
            return Err(e);
        }
        let result = EmptyClipboard()
            .map_err(|e| format!("no se pudo vaciar el portapapeles: {e}"))
            .and_then(|()| {
                SetClipboardData(u32::from(CF_HDROP.0), Some(HANDLE(memory.0)))
                    .map(|_| ())
                    .map_err(|e| format!("no se pudo copiar el archivo al portapapeles: {e}"))
            });
        if result.is_err() {
            GlobalFree(Some(memory)).ok();
        }
        CloseClipboard().ok();
        result
    }
}

/// Copy `bytes` into the global block.
///
/// # Safety
/// `memory` must be a live block of at least `bytes.len()` bytes.
unsafe fn fill(memory: HGLOBAL, bytes: &[u8]) -> Result<(), String> {
    let locked = GlobalLock(memory);
    if locked.is_null() {
        return Err("no se pudo preparar la memoria del portapapeles".to_string());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked.cast::<u8>(), bytes.len());
    GlobalUnlock(memory).ok();
    Ok(())
}

fn open_clipboard() -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..OPEN_TRIES {
        // SAFETY: no window handle: the clipboard is tied to this task.
        match unsafe { OpenClipboard(None) } {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        sleep(OPEN_WAIT);
    }
    Err(format!(
        "otra aplicación tiene ocupado el portapapeles: {last}"
    ))
}
