use std::{
    fmt,
    sync::mpsc::{Receiver, TryRecvError},
};

#[cfg(any(target_os = "windows", test))]
use crate::app::dialogs::OutputFormat;
use crate::app::dialogs::{DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogRequest};
#[cfg(windows)]
use crate::native::WindowOwner;
#[cfg(any(windows, test))]
use std::{path::PathBuf, sync::mpsc};
#[cfg(test)]
use std::{
    sync::mpsc::{SyncSender, TrySendError},
    thread,
};

#[cfg(target_os = "windows")]
const MEDIA_FILTER_NAME: &str = "Video files";
#[cfg(target_os = "windows")]
const MEDIA_FILTER_PATTERN: &str = "*.mp4;*.mkv;*.mov;*.avi;*.webm;*.m4v";
#[cfg(target_os = "windows")]
const ALL_FILES_FILTER_NAME: &str = "All files";
#[cfg(target_os = "windows")]
const ALL_FILES_FILTER_PATTERN: &str = "*.*";

#[cfg(any(target_os = "windows", test))]
impl OutputFormat {
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Mka => "mka",
            Self::M4a => "m4a",
            Self::Flac => "flac",
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    #[cfg(target_os = "windows")]
    const fn filter_name(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 video (*.mp4)",
            Self::Mkv => "Matroska video (*.mkv)",
            Self::Mka => "Matroska audio (*.mka)",
            Self::M4a => "MPEG-4 audio (*.m4a)",
            Self::Flac => "FLAC audio (*.flac)",
            Self::Png => "PNG image (*.png)",
            Self::Jpeg => "JPEG image (*.jpg)",
        }
    }

    const fn filter_pattern(self) -> &'static str {
        match self {
            Self::Mp4 => "*.mp4",
            Self::Mkv => "*.mkv",
            Self::Mka => "*.mka",
            Self::M4a => "*.m4a",
            Self::Flac => "*.flac",
            Self::Png => "*.png",
            Self::Jpeg => "*.jpg",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileDialogStartError {
    AlreadyPending,
    #[cfg(windows)]
    WindowOwnerUnavailable(String),
    WorkerUnavailable(String),
}

impl fmt::Display for FileDialogStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyPending => formatter.write_str("a file dialog is already open"),
            #[cfg(windows)]
            Self::WindowOwnerUnavailable(diagnostic) => formatter.write_str(diagnostic),
            Self::WorkerUnavailable(diagnostic) => formatter.write_str(diagnostic),
        }
    }
}

type DialogDispatch = Box<dyn Fn(FileDialogRequest) -> Result<(), FileDialogStartError>>;

pub(crate) struct FileDialogOwner {
    dispatch: Option<DialogDispatch>,
    completion_receiver: Receiver<FileDialogCompletion>,
    pending_target: Option<DialogTarget>,
    unavailable: Option<FileDialogStartError>,
}

impl FileDialogOwner {
    #[cfg(windows)]
    pub(crate) fn new(
        wake: impl Fn() + Clone + Send + 'static,
        owner: Result<WindowOwner, String>,
        executor: gpui::ForegroundExecutor,
    ) -> Self {
        let owner = match owner {
            Ok(owner) => owner,
            Err(error) => {
                return Self::unavailable(FileDialogStartError::WindowOwnerUnavailable(error));
            }
        };
        let (sender, completion_receiver) = mpsc::sync_channel(1);
        Self {
            dispatch: Some(Box::new(move |request| {
                let sender = sender.clone();
                let wake = wake.clone();
                // Like GPUI's Windows prompt implementation, run the modal COM call on
                // the foreground executor, outside a borrowed App/Window/entity update.
                executor
                    .spawn(async move {
                        let mut runner = native_dialog_runner(owner);
                        let completion = run_request(&request, &mut runner);
                        if sender.send(completion).is_ok() {
                            wake();
                        }
                    })
                    .detach();
                Ok(())
            })),
            completion_receiver,
            pending_target: None,
            unavailable: None,
        }
    }

    #[cfg(test)]
    fn spawn<F, R>(wake: impl Fn() + Send + 'static, runner_factory: F) -> Self
    where
        F: FnOnce() -> Result<R, String> + Send + 'static,
        R: DialogBackend + Send + 'static,
    {
        let (request_sender, request_receiver) = mpsc::sync_channel(1);
        let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("apollo-file-dialog".to_owned())
            .spawn(move || {
                run_dialog_service(request_receiver, completion_sender, wake, runner_factory);
            }) {
            Ok(_) => Self {
                dispatch: Some(Box::new(move |request| {
                    match request_sender.try_send(request) {
                        Ok(()) => Ok(()),
                        Err(TrySendError::Full(_)) => Err(FileDialogStartError::AlreadyPending),
                        Err(TrySendError::Disconnected(_)) => {
                            Err(FileDialogStartError::WorkerUnavailable(
                                "The native file dialog service disconnected.".into(),
                            ))
                        }
                    }
                })),
                completion_receiver,
                pending_target: None,
                unavailable: None,
            },
            Err(error) => Self {
                dispatch: None,
                completion_receiver,
                pending_target: None,
                unavailable: Some(FileDialogStartError::WorkerUnavailable(error.to_string())),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test<F, R>(wake: impl Fn() + Send + 'static, runner_factory: F) -> Self
    where
        F: FnOnce() -> Result<R, String> + Send + 'static,
        R: DialogBackend + Send + 'static,
    {
        Self::spawn(wake, runner_factory)
    }

    #[cfg(windows)]
    fn unavailable(error: FileDialogStartError) -> Self {
        let (_, completion_receiver) = mpsc::sync_channel(1);
        Self {
            dispatch: None,
            completion_receiver,
            pending_target: None,
            unavailable: Some(error),
        }
    }

    pub(crate) fn request(
        &mut self,
        request: FileDialogRequest,
    ) -> Result<(), FileDialogStartError> {
        if self.pending_target.is_some() {
            return Err(FileDialogStartError::AlreadyPending);
        }
        if let Some(error) = self.unavailable.clone() {
            return Err(error);
        }

        let target = request.target();
        let Some(dispatch) = self.dispatch.as_ref() else {
            return Err(FileDialogStartError::WorkerUnavailable(
                "The native file dialog service is unavailable.".to_owned(),
            ));
        };
        dispatch(request)?;
        self.pending_target = Some(target);
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Option<FileDialogCompletion> {
        let target = self.pending_target?;
        match self.completion_receiver.try_recv() {
            Ok(completion) => {
                self.pending_target = None;
                Some(completion)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.pending_target = None;
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
        self.pending_target.is_some()
    }
}

#[cfg(test)]
fn run_dialog_service<F, R>(
    request_receiver: Receiver<FileDialogRequest>,
    completion_sender: SyncSender<FileDialogCompletion>,
    wake: impl Fn() + Send + 'static,
    runner_factory: F,
) where
    F: FnOnce() -> Result<R, String>,
    R: DialogBackend,
{
    let mut runner = runner_factory();
    while let Ok(request) = request_receiver.recv() {
        if completion_sender
            .send(run_request(&request, &mut runner))
            .is_err()
        {
            break;
        }
        wake();
    }
}

#[cfg(any(windows, test))]
fn run_request<R: DialogBackend>(
    request: &FileDialogRequest,
    runner: &mut Result<R, String>,
) -> FileDialogCompletion {
    let target = request.target();
    let outcome = match runner.as_mut() {
        Ok(runner) => {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if matches!(request, FileDialogRequest::OpenSources { .. }) {
                    runner.run_many(request).map(|paths| {
                        paths.map_or(DialogOutcome::Cancelled, DialogOutcome::SelectedMany)
                    })
                } else {
                    runner
                        .run(request)
                        .map(|path| path.map_or(DialogOutcome::Cancelled, DialogOutcome::Selected))
                }
            })) {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => DialogOutcome::Failed(error),
                Err(_) => {
                    DialogOutcome::Failed("The native file dialog stopped unexpectedly.".to_owned())
                }
            }
        }
        Err(error) => DialogOutcome::Failed(error.clone()),
    };
    FileDialogCompletion { target, outcome }
}

#[cfg(any(windows, test))]
pub(crate) trait DialogBackend {
    fn run(&mut self, request: &FileDialogRequest) -> Result<Option<PathBuf>, String>;

    fn run_many(&mut self, request: &FileDialogRequest) -> Result<Option<Vec<PathBuf>>, String> {
        self.run(request).map(|path| path.map(|path| vec![path]))
    }
}

#[cfg(test)]
impl<F> DialogBackend for F
where
    F: FnMut(&FileDialogRequest) -> Result<Option<PathBuf>, String>,
{
    fn run(&mut self, request: &FileDialogRequest) -> Result<Option<PathBuf>, String> {
        self(request)
    }
}

#[cfg(target_os = "windows")]
struct NativeDialogBackend(native_windows::DialogSession);

#[cfg(target_os = "windows")]
impl DialogBackend for NativeDialogBackend {
    fn run(&mut self, request: &FileDialogRequest) -> Result<Option<PathBuf>, String> {
        self.0.run(request).map_err(|error| error.to_string())
    }

    fn run_many(&mut self, request: &FileDialogRequest) -> Result<Option<Vec<PathBuf>>, String> {
        self.0.run_many(request).map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "windows")]
fn native_dialog_runner(owner: WindowOwner) -> Result<NativeDialogBackend, String> {
    native_windows::DialogSession::new(owner)
        .map(NativeDialogBackend)
        .map_err(|error| error.to_string())
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
                Common::COMDLG_FILTERSPEC, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
                FOS_FORCEFILESYSTEM, FOS_NOREADONLYRETURN, FOS_PATHMUSTEXIST, FOS_STRICTFILETYPES,
                FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IShellItem,
                SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
            },
        },
        core::{HRESULT, HSTRING, PCWSTR, Result},
    };

    use super::{
        ALL_FILES_FILTER_NAME, ALL_FILES_FILTER_PATTERN, FileDialogRequest, MEDIA_FILTER_NAME,
        MEDIA_FILTER_PATTERN, WindowOwner,
    };
    use crate::app::dialogs::SaveDialogOptions;

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

    pub(super) struct DialogSession {
        _apartment: ComApartment,
        owner: WindowOwner,
    }

    impl DialogSession {
        pub(super) fn new(owner: WindowOwner) -> Result<Self> {
            Ok(Self {
                _apartment: ComApartment::initialize()?,
                owner,
            })
        }

        pub(super) fn run(&mut self, request: &FileDialogRequest) -> Result<Option<PathBuf>> {
            match request {
                FileDialogRequest::OpenSources {
                    initial_directory, ..
                } => open_sources(initial_directory.as_deref(), self.owner)
                    .map(|paths| paths.and_then(|paths| paths.into_iter().next())),
                FileDialogRequest::SaveOutput { options, .. } => save_output(options, self.owner),
            }
        }

        pub(super) fn run_many(
            &mut self,
            request: &FileDialogRequest,
        ) -> Result<Option<Vec<PathBuf>>> {
            match request {
                FileDialogRequest::OpenSources {
                    initial_directory, ..
                } => open_sources(initial_directory.as_deref(), self.owner),
                _ => self.run(request).map(|path| path.map(|path| vec![path])),
            }
        }
    }

    fn open_sources(
        initial_directory: Option<&std::path::Path>,
        owner: WindowOwner,
    ) -> Result<Option<Vec<PathBuf>>> {
        let dialog: IFileOpenDialog =
            unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
        unsafe {
            dialog.SetTitle(&HSTRING::from("Open media files"))?;
            dialog.SetOptions(
                FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_ALLOWMULTISELECT,
            )?;
            set_filters(
                &dialog,
                &[
                    (MEDIA_FILTER_NAME, MEDIA_FILTER_PATTERN),
                    (ALL_FILES_FILTER_NAME, ALL_FILES_FILTER_PATTERN),
                ],
            )?;
            set_initial_folder_if_available(&dialog, initial_directory);
        }
        let shown =
            unsafe { dialog.Show(Some(HWND(owner.native_handle() as *mut core::ffi::c_void))) };
        if let Err(error) = shown {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }
        let items = unsafe { dialog.GetResults()? };
        let count = unsafe { items.GetCount()? };
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let item = unsafe { items.GetItemAt(index)? };
            paths.push(item_path(&item)?);
        }
        Ok(Some(paths))
    }

    fn save_output(options: &SaveDialogOptions, owner: WindowOwner) -> Result<Option<PathBuf>> {
        let dialog: IFileDialog =
            unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)? };
        unsafe {
            dialog.SetTitle(&HSTRING::from("Choose output destination"))?;
            // FOS_OVERWRITEPROMPT is intentionally omitted. Publication never replaces an existing file.
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

            set_initial_folder_if_available(&dialog, options.initial_directory.as_deref());
        }
        show(dialog, owner)
    }

    unsafe fn set_initial_folder_if_available(
        dialog: &IFileDialog,
        directory: Option<&std::path::Path>,
    ) {
        let Some(directory) = directory else {
            return;
        };
        let location = HSTRING::from(directory.as_os_str());
        let folder: Result<IShellItem> = unsafe { SHCreateItemFromParsingName(&location, None) };
        if let Ok(folder) = folder {
            let _ = unsafe { dialog.SetFolder(&folder) };
        }
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
        let owner = HWND(owner.native_handle() as *mut core::ffi::c_void);
        let shown = unsafe { dialog.Show(Some(owner)) };
        if let Err(error) = shown {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }

        let item = unsafe { dialog.GetResult()? };
        item_path(&item).map(Some)
    }

    fn item_path(item: &IShellItem) -> Result<PathBuf> {
        let display_name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path = unsafe { display_name.to_string() };
        unsafe { CoTaskMemFree(Some(display_name.0.cast())) };
        let path = path.map_err(|_| {
            windows::core::Error::new(E_FAIL, "The selected path was not valid UTF-16.")
        })?;
        Ok(PathBuf::from(path))
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    use super::{
        DialogBackend, DialogOutcome, DialogTarget, FileDialogOwner, FileDialogRequest,
        FileDialogStartError, OutputFormat,
    };

    use crate::app::dialogs::SaveDialogOptions;

    struct MultiSelectBackend(Vec<PathBuf>);

    impl DialogBackend for MultiSelectBackend {
        fn run(&mut self, _: &FileDialogRequest) -> Result<Option<PathBuf>, String> {
            Ok(None)
        }

        fn run_many(&mut self, _: &FileDialogRequest) -> Result<Option<Vec<PathBuf>>, String> {
            Ok(Some(self.0.clone()))
        }
    }

    const COMPLETION_TIMEOUT: Duration = Duration::from_secs(10);

    fn dialog_wake() -> (impl Fn() + Send + 'static, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        (
            move || {
                let _ = sender.send(());
            },
            receiver,
        )
    }

    fn open_request() -> FileDialogRequest {
        FileDialogRequest::OpenSources {
            initial_directory: Some(PathBuf::from(r"C:\media")),
            target: DialogTarget::BatchInput,
        }
    }

    #[test]
    fn open_request_routes_selected_path_through_owner() {
        let expected = PathBuf::from(r"C:\media\clip.mp4");
        let selected = expected.clone();
        let (callback, wake) = dialog_wake();
        let mut owner = FileDialogOwner::new_for_test(callback, move || {
            Ok(move |request: &FileDialogRequest| {
                assert_eq!(request, &open_request());
                Ok(Some(selected.clone()))
            })
        });
        owner.request(open_request()).unwrap();

        let completion = wait_for_completion(&mut owner, &wake);
        assert_eq!(completion.target, DialogTarget::BatchInput);
        assert_eq!(
            completion.outcome,
            DialogOutcome::SelectedMany(vec![expected])
        );
    }

    #[test]
    fn multi_open_request_preserves_every_selected_path_in_native_order() {
        let paths = vec![
            PathBuf::from(r"C:\media files\第一.mp4"),
            PathBuf::from(r"C:\media files\thứ hai.mkv"),
        ];
        let expected = paths.clone();
        let (callback, wake) = dialog_wake();
        let mut owner =
            FileDialogOwner::new_for_test(callback, move || Ok(MultiSelectBackend(paths)));
        owner
            .request(FileDialogRequest::OpenSources {
                initial_directory: Some(PathBuf::from(r"C:\media")),
                target: DialogTarget::BatchInput,
            })
            .unwrap();

        let completion = wait_for_completion(&mut owner, &wake);
        assert_eq!(completion.target, DialogTarget::BatchInput);
        assert_eq!(completion.outcome, DialogOutcome::SelectedMany(expected));
    }

    #[test]
    fn cancellation_is_not_a_failure() {
        let (callback, wake) = dialog_wake();
        let mut owner =
            FileDialogOwner::new_for_test(callback, || Ok(|_: &FileDialogRequest| Ok(None)));
        owner.request(open_request()).unwrap();

        assert_eq!(
            wait_for_completion(&mut owner, &wake).outcome,
            DialogOutcome::Cancelled
        );
    }

    #[test]
    fn save_request_preserves_destination_parameters() {
        let request = FileDialogRequest::SaveOutput {
            target: DialogTarget::AudioMixOutput { session_id: 7 },
            options: SaveDialogOptions {
                initial_directory: Some(PathBuf::from(r"C:\media")),
                file_name: "clip-mixed.mkv".into(),
                format: OutputFormat::Mkv,
            },
        };
        let expected = request.clone();
        let (callback, wake) = dialog_wake();
        let mut owner = FileDialogOwner::new_for_test(callback, move || {
            Ok(move |actual: &FileDialogRequest| {
                assert_eq!(actual, &expected);
                let FileDialogRequest::SaveOutput { options, .. } = actual else {
                    unreachable!();
                };
                assert_eq!(options.format.extension(), "mkv");
                assert_eq!(options.format.filter_pattern(), "*.mkv");
                Ok(None)
            })
        });
        owner.request(request).unwrap();

        assert_eq!(
            wait_for_completion(&mut owner, &wake).outcome,
            DialogOutcome::Cancelled
        );
    }

    #[test]
    fn audio_extract_formats_own_save_extensions_and_filters() {
        for (format, extension, pattern) in [
            (OutputFormat::Mka, "mka", "*.mka"),
            (OutputFormat::M4a, "m4a", "*.m4a"),
            (OutputFormat::Flac, "flac", "*.flac"),
        ] {
            assert_eq!(format.extension(), extension);
            assert_eq!(format.filter_pattern(), pattern);
        }
    }

    #[test]
    fn frame_capture_formats_own_save_extensions_and_filters() {
        for (format, extension, pattern) in [
            (OutputFormat::Png, "png", "*.png"),
            (OutputFormat::Jpeg, "jpg", "*.jpg"),
        ] {
            assert_eq!(format.extension(), extension);
            assert_eq!(format.filter_pattern(), pattern);
        }
    }

    #[test]
    fn owner_rejects_a_second_pending_request() {
        let (release_sender, release_receiver) = mpsc::channel();
        let mut release_receiver = Some(release_receiver);
        let (callback, wake) = dialog_wake();
        let mut owner = FileDialogOwner::new_for_test(callback, move || {
            Ok(move |_: &FileDialogRequest| {
                if let Some(receiver) = release_receiver.take() {
                    receiver.recv_timeout(COMPLETION_TIMEOUT).unwrap();
                }
                Ok(None)
            })
        });
        owner.request(open_request()).unwrap();

        assert!(owner.is_pending());
        assert_eq!(
            owner.request(open_request()),
            Err(FileDialogStartError::AlreadyPending)
        );
        release_sender.send(()).unwrap();
        let completion = wait_for_completion(&mut owner, &wake);
        assert_eq!(completion.outcome, DialogOutcome::Cancelled);
        assert!(owner.request(open_request()).is_ok());
    }

    #[test]
    fn backend_failure_remains_distinct_from_cancellation() {
        let (callback, wake) = dialog_wake();
        let mut owner = FileDialogOwner::new_for_test(callback, || {
            Ok(|_: &FileDialogRequest| Err("native dialog diagnostic".to_owned()))
        });
        owner.request(open_request()).unwrap();

        assert_eq!(
            wait_for_completion(&mut owner, &wake).outcome,
            DialogOutcome::Failed("native dialog diagnostic".to_owned())
        );
    }

    fn wait_for_completion(
        owner: &mut FileDialogOwner,
        wake: &mpsc::Receiver<()>,
    ) -> super::FileDialogCompletion {
        wake.recv_timeout(COMPLETION_TIMEOUT)
            .expect("dialog completion must notify the owner");
        let completion = owner
            .poll()
            .expect("dialog result must be published before notification");
        assert!(!owner.is_pending());
        assert!(owner.poll().is_none());
        completion
    }
}
