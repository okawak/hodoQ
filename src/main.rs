#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() -> anyhow::Result<()> {
    if let Err(error) = hodoq::run() {
        show_startup_error(&error.to_string());
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn show_startup_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    let message = message
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let title = "HodoQ 起動エラー"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both UTF-16 buffers are NUL-terminated and live for the synchronous call.
    unsafe {
        let _ = MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_startup_error(message: &str) {
    eprintln!("HodoQ: {message}");
}
