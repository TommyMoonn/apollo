use std::{
    ffi::OsString,
    fmt,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
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
    AudioExtractOutput { session_id: u64 },
    FrameCaptureOutput { session_id: u64 },
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Mp4,
    Mkv,
    Mka,
    M4a,
    Flac,
    Png,
    Jpeg,
}

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
pub(crate) struct SaveDialogOptions {
    pub(crate) initial_directory: Option<PathBuf>,
    pub(crate) file_name: OsString,
    pub(crate) format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileDialogRequest {
    OpenSource {
        initial_directory: Option<PathBuf>,
    },
    SaveOutput {
        target: DialogTarget,
        options: SaveDialogOptions,
    },
}

impl FileDialogRequest {
    pub(crate) fn target(&self) -> DialogTarget {
        match self {
            Self::OpenSource { .. } => DialogTarget::MediaInput,
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

pub(crate) struct FileDialogOwner {
    request_sender: Option<SyncSender<FileDialogRequest>>,
    completion_receiver: Receiver<FileDialogCompletion>,
    pending_target: Option<DialogTarget>,
    unavailable: Option<FileDialogStartError>,
}

impl FileDialogOwner {
    pub(crate) fn new(context: Context, owner: Result<WindowOwner, String>) -> Self {
        match owner {
            Ok(owner) => Self::spawn(context, move || native_dialog_runner(owner)),
            Err(error) => Self::unavailable(FileDialogStartError::WindowOwnerUnavailable(error)),
        }
    }

    fn spawn<F, R>(context: Context, runner_factory: F) -> Self
    where
        F: FnOnce() -> Result<R, String> + Send + 'static,
        R: DialogBackend + Send + 'static,
    {
        let (request_sender, request_receiver) = mpsc::sync_channel(1);
        let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("apollo-file-dialog".to_owned())
            .spawn(move || {
                run_dialog_service(request_receiver, completion_sender, context, runner_factory);
            }) {
            Ok(_) => Self {
                request_sender: Some(request_sender),
                completion_receiver,
                pending_target: None,
                unavailable: None,
            },
            Err(error) => Self {
                request_sender: None,
                completion_receiver,
                pending_target: None,
                unavailable: Some(FileDialogStartError::WorkerUnavailable(error.to_string())),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test<F, R>(context: Context, runner_factory: F) -> Self
    where
        F: FnOnce() -> Result<R, String> + Send + 'static,
        R: DialogBackend + Send + 'static,
    {
        Self::spawn(context, runner_factory)
    }

    fn unavailable(error: FileDialogStartError) -> Self {
        let (_, completion_receiver) = mpsc::sync_channel(1);
        Self {
            request_sender: None,
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
        let Some(sender) = self.request_sender.as_ref() else {
            return Err(FileDialogStartError::WorkerUnavailable(
                "The native file dialog service is unavailable.".to_owned(),
            ));
        };
        match sender.try_send(request) {
            Ok(()) => {
                self.pending_target = Some(target);
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(FileDialogStartError::AlreadyPending),
            Err(TrySendError::Disconnected(_)) => Err(FileDialogStartError::WorkerUnavailable(
                "The native file dialog service disconnected.".to_owned(),
            )),
        }
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

fn run_dialog_service<F, R>(
    request_receiver: Receiver<FileDialogRequest>,
    completion_sender: SyncSender<FileDialogCompletion>,
    context: Context,
    runner_factory: F,
) where
    F: FnOnce() -> Result<R, String>,
    R: DialogBackend,
{
    let mut runner = runner_factory();
    while let Ok(request) = request_receiver.recv() {
        let target = request.target();
        let outcome = match runner.as_mut() {
            Ok(runner) => {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runner.run(&request)
                })) {
                    Ok(Ok(Some(path))) => DialogOutcome::Selected(path),
                    Ok(Ok(None)) => DialogOutcome::Cancelled,
                    Ok(Err(error)) => DialogOutcome::Failed(error),
                    Err(_) => DialogOutcome::Failed(
                        "The native file dialog stopped unexpectedly.".to_owned(),
                    ),
                }
            }
            Err(error) => DialogOutcome::Failed(error.clone()),
        };
        if completion_sender
            .send(FileDialogCompletion { target, outcome })
            .is_err()
        {
            break;
        }
        context.request_repaint();
    }
}

pub(crate) trait DialogBackend {
    fn run(&mut self, request: &FileDialogRequest) -> Result<Option<PathBuf>, String>;
}

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
}

#[cfg(target_os = "windows")]
fn native_dialog_runner(owner: WindowOwner) -> Result<NativeDialogBackend, String> {
    native_windows::DialogSession::new(owner)
        .map(NativeDialogBackend)
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
fn native_dialog_runner(
    _owner: WindowOwner,
) -> Result<fn(&FileDialogRequest) -> Result<Option<PathBuf>, String>, String> {
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
                FileDialogRequest::OpenSource { initial_directory } => {
                    open_source(initial_directory.as_deref(), self.owner)
                }
                FileDialogRequest::SaveOutput { options, .. } => save_output(options, self.owner),
            }
        }
    }

    fn open_source(
        initial_directory: Option<&std::path::Path>,
        owner: WindowOwner,
    ) -> Result<Option<PathBuf>> {
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
            set_initial_folder_if_available(&dialog, initial_directory);
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
    use std::{
        collections::VecDeque,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread::ThreadId,
        time::Duration,
    };

    use super::{
        DialogOutcome, DialogTarget, FileDialogOwner, FileDialogRequest, FileDialogStartError,
        OutputFormat, SaveDialogOptions,
    };

    fn open_request() -> FileDialogRequest {
        FileDialogRequest::OpenSource {
            initial_directory: Some(PathBuf::from(r"C:\media")),
        }
    }

    #[test]
    fn open_request_routes_selected_path_through_owner() {
        let expected = PathBuf::from(r"C:\media\clip.mp4");
        let selected = expected.clone();
        let mut owner = FileDialogOwner::new_for_test(egui::Context::default(), move || {
            Ok(move |request: &FileDialogRequest| {
                assert_eq!(request, &open_request());
                Ok(Some(selected.clone()))
            })
        });
        owner.request(open_request()).unwrap();

        let completion = wait_for_completion(&mut owner);
        assert_eq!(completion.target, DialogTarget::MediaInput);
        assert_eq!(completion.outcome, DialogOutcome::Selected(expected));
    }

    #[test]
    fn cancellation_is_not_a_failure() {
        let mut owner = FileDialogOwner::new_for_test(egui::Context::default(), || {
            Ok(|_: &FileDialogRequest| Ok(None))
        });
        owner.request(open_request()).unwrap();

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
                initial_directory: Some(PathBuf::from(r"C:\media")),
                file_name: "clip-mixed.mkv".into(),
                format: OutputFormat::Mkv,
            },
        };
        let expected = request.clone();
        let mut owner = FileDialogOwner::new_for_test(egui::Context::default(), move || {
            Ok(move |actual: &FileDialogRequest| {
                assert_eq!(actual, &expected);
                let FileDialogRequest::SaveOutput { options, .. } = actual else {
                    unreachable!();
                };
                assert_eq!(options.format.extension(), "mkv");
                assert_eq!(options.format.filter_name(), "Matroska video (*.mkv)");
                assert_eq!(options.format.filter_pattern(), "*.mkv");
                Ok(None)
            })
        });
        owner.request(request).unwrap();

        assert_eq!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Cancelled
        );
    }

    #[test]
    fn audio_extract_formats_own_save_extensions_and_filters() {
        for (format, extension, name, pattern) in [
            (OutputFormat::Mka, "mka", "Matroska audio (*.mka)", "*.mka"),
            (OutputFormat::M4a, "m4a", "MPEG-4 audio (*.m4a)", "*.m4a"),
            (OutputFormat::Flac, "flac", "FLAC audio (*.flac)", "*.flac"),
        ] {
            assert_eq!(format.extension(), extension);
            assert_eq!(format.filter_name(), name);
            assert_eq!(format.filter_pattern(), pattern);
        }
    }

    #[test]
    fn frame_capture_formats_own_save_extensions_and_filters() {
        for (format, extension, name, pattern) in [
            (OutputFormat::Png, "png", "PNG image (*.png)", "*.png"),
            (OutputFormat::Jpeg, "jpg", "JPEG image (*.jpg)", "*.jpg"),
        ] {
            assert_eq!(format.extension(), extension);
            assert_eq!(format.filter_name(), name);
            assert_eq!(format.filter_pattern(), pattern);
        }
    }

    #[test]
    fn owner_rejects_a_second_pending_request() {
        let (release_sender, release_receiver) = mpsc::channel();
        let mut release_receiver = Some(release_receiver);
        let mut owner = FileDialogOwner::new_for_test(egui::Context::default(), move || {
            Ok(move |_: &FileDialogRequest| {
                if let Some(receiver) = release_receiver.take() {
                    receiver.recv().unwrap();
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
        let _ = wait_for_completion(&mut owner);
    }

    #[test]
    fn backend_failure_remains_distinct_from_cancellation() {
        let mut owner = FileDialogOwner::new_for_test(egui::Context::default(), || {
            Ok(|_: &FileDialogRequest| Err("native dialog diagnostic".to_owned()))
        });
        owner.request(open_request()).unwrap();

        assert_eq!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Failed("native dialog diagnostic".to_owned())
        );
    }

    #[test]
    fn repeated_requests_reuse_one_initialized_service_thread() {
        let initialization_count = Arc::new(AtomicUsize::new(0));
        let thread_ids = Arc::new(Mutex::new(Vec::<ThreadId>::new()));
        let results = VecDeque::from([
            Some(PathBuf::from(r"C:\media\first.mp4")),
            Some(PathBuf::from(r"C:\output\second.mkv")),
        ]);
        let mut results = results;
        let initialized = Arc::clone(&initialization_count);
        let observed_threads = Arc::clone(&thread_ids);
        let mut owner = FileDialogOwner::new_for_test(egui::Context::default(), move || {
            initialized.fetch_add(1, Ordering::SeqCst);
            Ok(move |_: &FileDialogRequest| {
                observed_threads
                    .lock()
                    .unwrap()
                    .push(std::thread::current().id());
                Ok(results.pop_front().unwrap())
            })
        });

        owner.request(open_request()).unwrap();
        assert!(matches!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Selected(_)
        ));
        owner.request(open_request()).unwrap();
        assert!(matches!(
            wait_for_completion(&mut owner).outcome,
            DialogOutcome::Selected(_)
        ));

        let thread_ids = thread_ids.lock().unwrap();
        assert_eq!(initialization_count.load(Ordering::SeqCst), 1);
        assert_eq!(thread_ids.len(), 2);
        assert_eq!(thread_ids[0], thread_ids[1]);
    }

    #[test]
    fn completion_requests_egui_repaint() {
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });
        let mut owner =
            FileDialogOwner::new_for_test(context, || Ok(|_: &FileDialogRequest| Ok(None)));

        owner.request(open_request()).unwrap();
        let _ = wait_for_completion(&mut owner);
        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("dialog completion should wake egui");
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
