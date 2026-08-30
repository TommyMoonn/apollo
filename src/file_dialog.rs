use std::{
    ffi::OsString,
    fmt,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use egui::Context;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

const MEDIA_FILTER_NAME: &str = "Video files";
const MEDIA_FILTER_PATTERN: &str = "*.mp4;*.mkv;*.mov;*.avi;*.webm;*.m4v";
const ALL_FILES_FILTER_NAME: &str = "All files";
const ALL_FILES_FILTER_PATTERN: &str = "*.*";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogTarget {
    MediaInput,
    AudioMixOutput { session_id: u64 },
    UpscaleOutput { session_id: u64 },
    RemuxOutput { session_id: u64 },
}

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

    #[cfg(test)]
    pub(crate) fn test_value() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            hwnd: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Mp4,
    Mkv,
}

impl OutputFormat {
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
        }
    }

    const fn filter_name(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 video (*.mp4)",
            Self::Mkv => "Matroska video (*.mkv)",
        }
    }

    const fn filter_pattern(self) -> &'static str {
        match self {
            Self::Mp4 => "*.mp4",
            Self::Mkv => "*.mkv",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SaveDialogOptions {
    pub(crate) initial_directory: PathBuf,
    pub(crate) file_name: OsString,
    pub(crate) format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileDialogRequest {
    OpenMedia,
    SaveOutput {
        target: DialogTarget,
        options: SaveDialogOptions,
    },
}

impl FileDialogRequest {
    pub(crate) fn target(&self) -> DialogTarget {
        match self {
            Self::OpenMedia => DialogTarget::MediaInput,
            Self::SaveOutput { target, .. } => *target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    Selected(PathBuf),
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileDialogCompletion {
    pub(crate) target: DialogTarget,
    pub(crate) outcome: DialogOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileDialogStartError {
    AlreadyPending,
    WindowOwnerUnavailable(String),
    WorkerUnavailable(String),
}

impl fmt::Display for FileDialogStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyPending => formatter.write_str("a file dialog is already open"),
            Self::WindowOwnerUnavailable(diagnostic) => formatter.write_str(diagnostic),
            Self::WorkerUnavailable(diagnostic) => formatter.write_str(diagnostic),
        }
    }
}

#[derive(Default)]
pub(crate) struct FileDialogOwner {
    pending: Option<PendingDialog>,
}

struct PendingDialog {
    target: DialogTarget,
    receiver: Receiver<FileDialogCompletion>,
}

impl FileDialogOwner {
    pub(crate) fn request(
        &mut self,
        request: FileDialogRequest,
        context: Context,
        owner: Result<WindowOwner, String>,
    ) -> Result<(), FileDialogStartError> {
        if self.pending.is_some() {
            return Err(FileDialogStartError::AlreadyPending);
        }
        let owner = owner.map_err(FileDialogStartError::WindowOwnerUnavailable)?;
        self.request_with(request, context, move |request| {
            run_native_dialog(request, owner)
        })
    }

    fn request_with<F>(
        &mut self,
        request: FileDialogRequest,
        context: Context,
        runner: F,
    ) -> Result<(), FileDialogStartError>
    where
        F: FnOnce(&FileDialogRequest) -> Result<Option<PathBuf>, String> + Send + 'static,
    {
        if self.pending.is_some() {
            return Err(FileDialogStartError::AlreadyPending);
        }

        let target = request.target();
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-file-dialog".to_owned())
            .spawn(move || {
                let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runner(&request)
                })) {
                    Ok(Ok(Some(path))) => DialogOutcome::Selected(path),
                    Ok(Ok(None)) => DialogOutcome::Cancelled,
                    Ok(Err(error)) => DialogOutcome::Failed(error),
                    Err(_) => DialogOutcome::Failed(
                        "The native file dialog stopped unexpectedly.".to_owned(),
                    ),
                };
                let _ = sender.send(FileDialogCompletion { target, outcome });
                context.request_repaint();
            })
            .map_err(|error| FileDialogStartError::WorkerUnavailable(error.to_string()))?;

        self.pending = Some(PendingDialog { target, receiver });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn request_for_test<F>(
        &mut self,
        request: FileDialogRequest,
        context: Context,
        runner: F,
    ) -> Result<(), FileDialogStartError>
    where
        F: FnOnce(&FileDialogRequest) -> Result<Option<PathBuf>, String> + Send + 'static,
    {
        self.request_with(request, context, runner)
    }

    pub(crate) fn poll(&mut self) -> Option<FileDialogCompletion> {
        let pending = self.pending.as_ref()?;
        let target = pending.target;
        match pending.receiver.try_recv() {
            Ok(completion) => {
                self.pending = None;
                Some(completion)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                Some(FileDialogCompletion {
                    target,
                    outcome: DialogOutcome::Failed(
                        "The native file dialog worker disconnected.".to_owned(),
                    ),
                })
            }
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(target_os = "windows")]
fn run_native_dialog(
    request: &FileDialogRequest,
    owner: WindowOwner,
) -> Result<Option<PathBuf>, String> {
    native_windows::run(request, owner).map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
fn run_native_dialog(
    _request: &FileDialogRequest,
    _owner: WindowOwner,
) -> Result<Option<PathBuf>, String> {
    Err("Native file dialogs are currently available on Windows.".to_owned())
}

#[cfg(target_os = "windows")]
mod native_windows {
    use std::path::PathBuf;

    use windows::{
        Win32::{
            Foundation::{E_FAIL, ERROR_CANCELLED, HWND},
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            UI::Shell::{
                Common::COMDLG_FILTERSPEC, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
                FOS_NOREADONLYRETURN, FOS_PATHMUSTEXIST, FOS_STRICTFILETYPES, FileOpenDialog,
                FileSaveDialog, IFileDialog, IShellItem, SHCreateItemFromParsingName,
                SIGDN_FILESYSPATH,
            },
        },
        core::{HRESULT, HSTRING, PCWSTR, Result},
    };

    use super::{
        ALL_FILES_FILTER_NAME, ALL_FILES_FILTER_PATTERN, FileDialogRequest, MEDIA_FILTER_NAME,
        MEDIA_FILTER_PATTERN, SaveDialogOptions, WindowOwner,
    };

    struct ComApartment;

    impl ComApartment {
        fn initialize() -> Result<Self> {
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    pub(super) fn run(request: &FileDialogRequest, owner: WindowOwner) -> Result<Option<PathBuf>> {
        let _apartment = ComApartment::initialize()?;
        match request {
            FileDialogRequest::OpenMedia => open_media(owner),
            FileDialogRequest::SaveOutput { options, .. } => save_output(options, owner),
        }
    }

    fn open_media(owner: WindowOwner) -> Result<Option<PathBuf>> {
        let dialog: IFileDialog =
            unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
        unsafe {
            dialog.SetTitle(&HSTRING::from("Open media file"))?;
            dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)?;
            set_filters(
                &dialog,
                &[
                    (MEDIA_FILTER_NAME, MEDIA_FILTER_PATTERN),
                    (ALL_FILES_FILTER_NAME, ALL_FILES_FILTER_PATTERN),
                ],
            )?;
        }
        show(dialog, owner)
    }

    fn save_output(options: &SaveDialogOptions, owner: WindowOwner) -> Result<Option<PathBuf>> {
        let dialog: IFileDialog =
            unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)? };
        unsafe {
            dialog.SetTitle(&HSTRING::from("Choose output destination"))?;
            // FOS_OVERWRITEPROMPT is intentionally omitted. Apollo validates destinations.
            dialog.SetOptions(
                FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_NOREADONLYRETURN
                    | FOS_STRICTFILETYPES,
            )?;
            let format_filter = (
                options.format.filter_name(),
                options.format.filter_pattern(),
            );
            set_filters(
                &dialog,
                &[
                    format_filter,
                    (ALL_FILES_FILTER_NAME, ALL_FILES_FILTER_PATTERN),
                ],
            )?;
            dialog.SetDefaultExtension(&HSTRING::from(options.format.extension()))?;
            dialog.SetFileName(&HSTRING::from(options.file_name.as_os_str()))?;

            let location = HSTRING::from(options.initial_directory.as_os_str());
            let folder: IShellItem = SHCreateItemFromParsingName(&location, None)?;
            dialog.SetFolder(&folder)?;
        }
        show(dialog, owner)
    }

    unsafe fn set_filters(dialog: &IFileDialog, filters: &[(&str, &str)]) -> Result<()> {
        let names: Vec<HSTRING> = filters
            .iter()
            .map(|(name, _)| HSTRING::from(*name))
            .collect();
        let patterns: Vec<HSTRING> = filters
            .iter()
            .map(|(_, pattern)| HSTRING::from(*pattern))
            .collect();
        let specifications: Vec<COMDLG_FILTERSPEC> = names
            .iter()
            .zip(patterns.iter())
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            })
            .collect();
        unsafe { dialog.SetFileTypes(&specifications) }
    }

    fn show(dialog: IFileDialog, owner: WindowOwner) -> Result<Option<PathBuf>> {
        let owner = HWND(owner.hwnd as *mut core::ffi::c_void);
        let shown = unsafe { dialog.Show(Some(owner)) };
        if let Err(error) = shown {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }

        let item = unsafe { dialog.GetResult()? };
        let display_name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path = unsafe { display_name.to_string() };
        unsafe { CoTaskMemFree(Some(display_name.0.cast())) };
        let path = path.map_err(|_| {
            windows::core::Error::new(E_FAIL, "The selected path was not valid UTF-16.")
        })?;
        Ok(Some(PathBuf::from(path)))
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    use super::{
        DialogOutcome, DialogTarget, FileDialogOwner, FileDialogRequest, FileDialogStartError,
        OutputFormat, SaveDialogOptions,
    };

    #[test]
    fn open_request_routes_selected_path_through_owner() {
        let expected = PathBuf::from(r"C:\media\clip.mp4");
        let selected = expected.clone();
        let mut owner = FileDialogOwner::default();
        owner
            .request_with(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                move |request| {
                    assert_eq!(request, &FileDialogRequest::OpenMedia);
                    Ok(Some(selected))
                },
            )
            .unwrap();

        let completion = wait_for_completion(&mut owner);
        assert_eq!(completion.target, DialogTarget::MediaInput);
        assert_eq!(completion.outcome, DialogOutcome::Selected(expected));
    }

    #[test]
    fn cancellation_is_not_a_failure() {
        let mut owner = FileDialogOwner::default();
        owner
            .request_with(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                |_| Ok(None),
            )
            .unwrap();

        assert_eq!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Cancelled
        );
    }

    #[test]
    fn save_request_preserves_destination_parameters() {
        let request = FileDialogRequest::SaveOutput {
            target: DialogTarget::AudioMixOutput { session_id: 7 },
            options: SaveDialogOptions {
                initial_directory: PathBuf::from(r"C:\media"),
                file_name: "clip-mixed.mkv".into(),
                format: OutputFormat::Mkv,
            },
        };
        let expected = request.clone();
        let mut owner = FileDialogOwner::default();
        owner
            .request_with(request, egui::Context::default(), move |actual| {
                assert_eq!(actual, &expected);
                let FileDialogRequest::SaveOutput { options, .. } = actual else {
                    unreachable!();
                };
                assert_eq!(options.format.extension(), "mkv");
                assert_eq!(options.format.filter_name(), "Matroska video (*.mkv)");
                assert_eq!(options.format.filter_pattern(), "*.mkv");
                Ok(None)
            })
            .unwrap();

        assert_eq!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Cancelled
        );
    }

    #[test]
    fn owner_rejects_a_second_pending_request() {
        let (release_sender, release_receiver) = mpsc::channel();
        let mut owner = FileDialogOwner::default();
        owner
            .request_with(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                move |_| {
                    release_receiver.recv().unwrap();
                    Ok(None)
                },
            )
            .unwrap();

        assert!(owner.is_pending());
        assert_eq!(
            owner.request_with(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                |_| Ok(None)
            ),
            Err(FileDialogStartError::AlreadyPending)
        );
        release_sender.send(()).unwrap();
        let _ = wait_for_completion(&mut owner);
    }

    #[test]
    fn backend_failure_remains_distinct_from_cancellation() {
        let mut owner = FileDialogOwner::default();
        owner
            .request_with(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                |_| Err("native dialog diagnostic".to_owned()),
            )
            .unwrap();

        assert_eq!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Failed("native dialog diagnostic".to_owned())
        );
    }

    fn wait_for_completion(owner: &mut FileDialogOwner) -> super::FileDialogCompletion {
        for _ in 0..100 {
            if let Some(completion) = owner.poll() {
                return completion;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("file dialog worker did not complete");
    }
}
