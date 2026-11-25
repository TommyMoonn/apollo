//! Native window handles and shell actions shared by presentation adapters.
use raw_window_handle::HasWindowHandle;
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;
use std::{io, path::Path, process::Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowOwner {
    #[cfg(target_os = "windows")]
    hwnd: isize,
}

impl WindowOwner {
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "Native GPUI entry is Windows-only")
    )]
    pub(crate) fn from_window(window: &impl HasWindowHandle) -> Result<Self, String> {
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        match handle.as_raw() {
            #[cfg(target_os = "windows")]
            RawWindowHandle::Win32(handle) => Ok(Self {
                hwnd: handle.hwnd.get(),
            }),
            _ => Err("Apollo's native window handle is unavailable on this platform.".to_owned()),
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) const fn native_handle(self) -> isize {
        self.hwnd
    }
}

pub(crate) fn open_folder(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    #[cfg(any(target_os = "windows", target_os = "macos", unix))]
    {
        command.arg(path).spawn().map(|_| ())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "opening folders is not supported on this platform",
        ))
    }
}

/// Use the executable's existing icon resource for the native GPUI window.
#[cfg(windows)]
pub(crate) fn set_application_icon(window: &impl HasWindowHandle) -> Result<(), String> {
    use std::ptr;
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{ICON_BIG, ICON_SMALL, LoadIconW, SendMessageW, WM_SETICON},
    };

    let owner = WindowOwner::from_window(window)?;
    // Resource 1 is embedded by assets/apollo.rc. LoadIconW returns a shared icon;
    // Windows retains it for the module lifetime and it must not be destroyed.
    // The window is live and belongs to this UI thread during initialization.
    unsafe {
        let module = GetModuleHandleW(ptr::null());
        if module.is_null() {
            return Err(format!(
                "Could not locate Apollo's executable: {}",
                io::Error::last_os_error()
            ));
        }
        // Win32 interprets this value as a resource ID, not a memory address.
        let icon = LoadIconW(module, ptr::without_provenance::<u16>(1));
        if icon.is_null() {
            return Err(format!(
                "Could not load Apollo's window icon: {}",
                io::Error::last_os_error()
            ));
        }
        let hwnd = owner.native_handle() as _;
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
    }
    Ok(())
}

/// Release builds have no console, so also show recoverable startup errors natively.
#[cfg(windows)]
pub(crate) fn report_startup_error(message: &str) {
    use std::ptr;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    eprintln!("{message}");
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = "Apollo".encode_utf16().chain([0]).collect();
    // Both NUL-terminated strings remain valid for the synchronous native dialog.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
