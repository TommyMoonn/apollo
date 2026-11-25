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
