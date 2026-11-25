//! Native window handles and shell actions shared by presentation adapters.
use raw_window_handle::HasWindowHandle;
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;
#[cfg(not(windows))]
use std::process::Command;
use std::{io, path::Path};

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

/// Outcome actions operate on the recorded published file, never the requested destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputAction {
    Open,
    Reveal,
}

impl OutputAction {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "Open output",
            Self::Reveal => "Show in folder",
        }
    }

    pub(crate) fn run(self, path: &Path) -> io::Result<()> {
        self.run_with(path, launch_output)
    }

    fn run_with(
        self,
        path: &Path,
        launch: impl FnOnce(Self, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        if !path.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "The output file is unavailable",
            ));
        }
        launch(self, path)
    }
}

#[cfg(windows)]
fn launch_output(action: OutputAction, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            System::Com::{
                COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            },
            UI::{
                Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteW},
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        },
        core::{PCWSTR, w},
    };

    let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // Shell extensions require COM. GPUI already initializes OLE on this thread;
    // a successful initialization (including S_FALSE) must still be balanced.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }.map_err(io::Error::other)?;
    let result = match action {
        OutputAction::Open => {
            let result = unsafe {
                ShellExecuteW(
                    None,
                    w!("open"),
                    PCWSTR(path.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                )
            };
            if result.0 as isize <= 32 {
                Err(io::Error::other(format!(
                    "Windows shell returned error {}",
                    result.0 as isize
                )))
            } else {
                Ok(())
            }
        }
        OutputAction::Reveal => {
            let mut item = std::ptr::null_mut();
            match unsafe { SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut item, 0, None) } {
                Ok(()) => {
                    // With no child list, the absolute item ID selects the file in its parent.
                    let result = unsafe { SHOpenFolderAndSelectItems(item, None, 0) };
                    unsafe { CoTaskMemFree(Some(item.cast())) };
                    result.map_err(io::Error::other)
                }
                Err(error) => Err(io::Error::other(error)),
            }
        }
    };
    unsafe { CoUninitialize() };
    result
}

#[cfg(not(windows))]
fn launch_output(action: OutputAction, path: &Path) -> io::Result<()> {
    // The retained non-Windows adapter opens the containing folder; Explorer selects the actual file.
    let path = match action {
        OutputAction::Open => path,
        OutputAction::Reveal => path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "The output directory is unavailable",
            )
        })?,
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut command = Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
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

/// Read the Windows animation preference once at startup. GPUI applies this to its animations.
#[cfg(windows)]
pub(crate) fn reduce_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: i32 = 1;
    // The query writes one Win32 BOOL into the supplied buffer.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    ok != 0 && enabled == 0
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

#[cfg(test)]
mod tests {
    #[test]
    fn output_actions_validate_at_activation_and_keep_the_published_file() {
        use super::OutputAction;
        use std::io;

        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("export (1).mkv");
        std::fs::write(&output, b"published").unwrap();
        for action in [OutputAction::Open, OutputAction::Reveal] {
            action
                .run_with(&output, |received, path| {
                    assert_eq!(received, action);
                    assert_eq!(path, output);
                    Ok(())
                })
                .unwrap();
            let failure = action
                .run_with(&output, |_, _| {
                    Err(io::Error::other("shell rejected the action"))
                })
                .unwrap_err();
            assert_eq!(failure.to_string(), "shell rejected the action");
            assert_eq!(std::fs::read(&output).unwrap(), b"published");
            for missing in [root.path().join("missing.mkv"), root.path().to_path_buf()] {
                assert_eq!(
                    action
                        .run_with(&missing, |_, _| panic!("must not launch without a file"))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::NotFound
                );
            }
        }
    }
}
