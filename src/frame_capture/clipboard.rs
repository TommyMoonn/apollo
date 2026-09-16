use std::{
    fmt,
    io::Cursor,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use crate::media::{
    error::{MediaError, MediaOutputContainer},
    execution::{MediaExecutionFailure, ProcessControl},
    intake::SourceIdentity,
    model::MediaFile,
    process::{
        MediaProcess, MediaProcessCommand, MediaProcessFailure, MediaProcessStartError,
        MediaProcessState, MediaProcessWake,
    },
    tools::MediaTools,
};

use super::{
    command::build_copy_frame_invocation, model::FramePosition, validation::validate_frame_position,
};

const COPY_FRAME_MAX_RGBA_BYTES: usize = 256 * 1024 * 1024;
const COPY_FRAME_DECODE_ALLOCATION_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const PNG_OVERHEAD_ALLOWANCE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFrameRequest {
    pub source_identity: SourceIdentity,
    pub input: PathBuf,
    pub video_stream_index: u32,
    pub position: FramePosition,
    pub source_size: [u32; 2],
    stdout_limit: usize,
}

impl CopyFrameRequest {
    pub fn new(
        source_identity: &SourceIdentity,
        media: &MediaFile,
        position: FramePosition,
    ) -> Result<Self, CopyFrameError> {
        let source = validated_copy_source(media, position)?;

        Ok(Self {
            source_identity: source_identity.clone(),
            input: media.path.clone(),
            video_stream_index: source.video_stream_index,
            position,
            source_size: source.size,
            stdout_limit: source.stdout_limit,
        })
    }

    pub fn stdout_limit(&self) -> usize {
        self.stdout_limit
    }
}

pub fn copy_frame_action_eligible(media: &MediaFile, position: FramePosition) -> bool {
    selected_copy_source(media, position).is_ok()
}

struct ValidatedCopySource {
    video_stream_index: u32,
    size: [u32; 2],
    stdout_limit: usize,
}

fn validated_copy_source(
    media: &MediaFile,
    position: FramePosition,
) -> Result<ValidatedCopySource, CopyFrameError> {
    let (video_stream_index, size) = selected_copy_source(media, position)?;
    let rgba_bytes = checked_rgba_bytes(size)?;
    let stdout_limit = rgba_bytes
        .checked_add(size[1] as usize)
        .and_then(|bytes| bytes.checked_add(PNG_OVERHEAD_ALLOWANCE_BYTES))
        .ok_or(CopyFrameError::FrameTooLarge {
            size,
            maximum_bytes: COPY_FRAME_MAX_RGBA_BYTES,
        })?;
    Ok(ValidatedCopySource {
        video_stream_index,
        size,
        stdout_limit,
    })
}

fn selected_copy_source(
    media: &MediaFile,
    position: FramePosition,
) -> Result<(u32, [u32; 2]), CopyFrameError> {
    validate_frame_position(position, media.duration)
        .map_err(|error| CopyFrameError::InvalidPosition(error.to_string()))?;
    let video = media
        .video_streams
        .first()
        .ok_or(CopyFrameError::MissingVideoStream)?;
    let size = [
        video.width.ok_or(CopyFrameError::MissingDimensions)?,
        video.height.ok_or(CopyFrameError::MissingDimensions)?,
    ];
    if size.contains(&0) {
        return Err(CopyFrameError::MissingDimensions);
    }
    Ok((video.stream_index, size))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFrameImage {
    size: [u32; 2],
    rgba: Vec<u8>,
}

impl CopyFrameImage {
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyFrameError {
    MissingVideoStream,
    MissingDimensions,
    InvalidPosition(String),
    FrameTooLarge {
        size: [u32; 2],
        maximum_bytes: usize,
    },
    Start(MediaProcessStartError),
    Process {
        reason: MediaProcessFailure,
        diagnostic: String,
    },
    Decode(String),
    Clipboard(String),
    WorkerStart(std::io::ErrorKind),
    WorkerStopped,
    AlreadyRunning,
    Cancelled,
}

impl CopyFrameError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::MissingVideoStream => MediaError::missing_stream(self.to_string()),
            Self::FrameTooLarge { .. } => MediaError::frame_too_large(self.to_string()),
            Self::Start(error) => MediaError::from_process_start_error(error),
            Self::Process { reason, diagnostic } => MediaError::from_execution_failure(
                &MediaExecutionFailure::Process {
                    reason: reason.clone(),
                    diagnostic: diagnostic.clone(),
                },
                MediaOutputContainer::Other,
            ),
            Self::Cancelled => MediaError::cancelled(),
            Self::MissingDimensions
            | Self::InvalidPosition(_)
            | Self::Decode(_)
            | Self::Clipboard(_)
            | Self::WorkerStart(_)
            | Self::WorkerStopped
            | Self::AlreadyRunning => MediaError::clipboard_unavailable(self.to_string()),
        }
    }
}

impl fmt::Display for CopyFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => formatter.write_str("the source has no video stream"),
            Self::MissingDimensions => {
                formatter.write_str("the source video dimensions are unavailable")
            }
            Self::InvalidPosition(reason) => {
                write!(formatter, "the selected frame is invalid: {reason}")
            }
            Self::FrameTooLarge {
                size,
                maximum_bytes,
            } => write!(
                formatter,
                "the {} x {} frame exceeds the safe {} MiB decoded-image limit",
                size[0],
                size[1],
                maximum_bytes / (1024 * 1024)
            ),
            Self::Start(error) => write!(formatter, "copy-frame process could not start: {error}"),
            Self::Process { reason, .. } => {
                write!(formatter, "copy-frame process failed: {reason:?}")
            }
            Self::Decode(reason) => {
                write!(formatter, "copied frame could not be decoded: {reason}")
            }
            Self::Clipboard(reason) => write!(formatter, "clipboard publication failed: {reason}"),
            Self::WorkerStart(kind) => {
                write!(formatter, "copy-frame worker could not start ({kind:?})")
            }
            Self::WorkerStopped => formatter.write_str("copy-frame worker stopped unexpectedly"),
            Self::AlreadyRunning => formatter.write_str("a frame copy is already running"),
            Self::Cancelled => formatter.write_str("frame copy was cancelled"),
        }
    }
}

impl std::error::Error for CopyFrameError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFrameCompletion {
    pub request: CopyFrameRequest,
    pub result: Result<Vec<u8>, CopyFrameError>,
}

pub struct CopyFrameExtraction {
    request: CopyFrameRequest,
    process: Option<Box<dyn CopyFrameProcessControl>>,
}

#[derive(Debug, Default)]
pub struct CopyFrameExtractionOwner {
    active: Option<CopyFrameExtraction>,
}

impl CopyFrameExtractionOwner {
    pub fn start(
        &mut self,
        tools: &MediaTools,
        request: CopyFrameRequest,
        wake: MediaProcessWake,
    ) -> Result<(), CopyFrameError> {
        if self.active.is_some() {
            return Err(CopyFrameError::AlreadyRunning);
        }
        self.active = Some(
            CopyFrameExtraction::start_with_wake(tools, request, wake)
                .map_err(CopyFrameError::Start)?,
        );
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn poll(&mut self) -> Option<CopyFrameCompletion> {
        let completion = self.active.as_mut()?.poll()?;
        self.active = None;
        Some(completion)
    }

    pub fn cancel(&mut self) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        active.cancel()
    }
}

trait CopyFrameProcessControl: ProcessControl {
    fn captured_stdout_bytes(&self) -> Vec<u8>;
}

impl CopyFrameProcessControl for MediaProcess {
    fn captured_stdout_bytes(&self) -> Vec<u8> {
        MediaProcess::captured_stdout_bytes(self)
    }
}

impl fmt::Debug for CopyFrameExtraction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CopyFrameExtraction")
            .field("request", &self.request)
            .field("running", &self.process.is_some())
            .finish()
    }
}

impl CopyFrameExtraction {
    pub fn start_with_wake(
        tools: &MediaTools,
        request: CopyFrameRequest,
        wake: MediaProcessWake,
    ) -> Result<Self, MediaProcessStartError> {
        Self::start_with_launcher(tools, request, move |command| {
            MediaProcess::start_with_wake(command, wake)
        })
    }

    pub fn cancel(&self) -> bool {
        self.process
            .as_ref()
            .is_some_and(|process| process.cancel())
    }

    pub fn poll(&mut self) -> Option<CopyFrameCompletion> {
        let state = self.process.as_ref()?.state();
        if state == MediaProcessState::Running {
            return None;
        }
        let process = self
            .process
            .take()
            .expect("copy-frame process exists while terminal output is settled");
        let result = match state {
            MediaProcessState::Completed => Ok(process.captured_stdout_bytes()),
            MediaProcessState::Failed(reason) => Err(CopyFrameError::Process {
                reason,
                diagnostic: process.diagnostic_stderr(),
            }),
            MediaProcessState::Cancelled => Err(CopyFrameError::Cancelled),
            MediaProcessState::Running => unreachable!("running copy-frame work returned above"),
        };
        Some(CopyFrameCompletion {
            request: self.request.clone(),
            result,
        })
    }

    fn start_with_launcher<F, P>(
        tools: &MediaTools,
        request: CopyFrameRequest,
        launcher: F,
    ) -> Result<Self, MediaProcessStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: CopyFrameProcessControl + 'static,
    {
        let invocation = build_copy_frame_invocation(&request);
        let stdout_limit = request.stdout_limit();
        let process = launcher(
            MediaProcessCommand::new(&tools.ffmpeg)
                .args(invocation.args)
                .captured_stdout_limit(stdout_limit),
        )?;
        Ok(Self {
            request,
            process: Some(Box::new(process)),
        })
    }
}

pub struct CopyFrameWorker {
    pub request: CopyFrameRequest,
    receiver: Receiver<Result<(), CopyFrameError>>,
}

impl CopyFrameWorker {
    pub fn spawn(
        request: CopyFrameRequest,
        png_bytes: Vec<u8>,
        native_window: Option<isize>,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<Self, CopyFrameError> {
        Self::spawn_with(request, wake, move |request| {
            let image = decode_copy_frame_png(&png_bytes, request.source_size)?;
            publish_image_to_clipboard(&image, native_window)
        })
    }

    fn spawn_with<F>(
        request: CopyFrameRequest,
        wake: impl FnOnce() + Send + 'static,
        work: F,
    ) -> Result<Self, CopyFrameError>
    where
        F: FnOnce(&CopyFrameRequest) -> Result<(), CopyFrameError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_request = request.clone();
        thread::Builder::new()
            .name("apollo-copy-frame".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&worker_request)
                }))
                .unwrap_or(Err(CopyFrameError::WorkerStopped));
                let _ = sender.send(result);
                wake();
            })
            .map_err(|error| CopyFrameError::WorkerStart(error.kind()))?;
        Ok(Self { request, receiver })
    }

    pub fn poll(&self) -> Option<Result<(), CopyFrameError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(CopyFrameError::WorkerStopped)),
        }
    }
}

pub fn decode_copy_frame_png(
    bytes: &[u8],
    expected_size: [u32; 2],
) -> Result<CopyFrameImage, CopyFrameError> {
    checked_rgba_bytes(expected_size)?;
    let maximum_edge = expected_size[0].max(expected_size[1]);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(maximum_edge);
    limits.max_image_height = Some(maximum_edge);
    limits.max_alloc = Some(COPY_FRAME_DECODE_ALLOCATION_LIMIT_BYTES);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|error| CopyFrameError::Decode(error.to_string()))?
        .to_rgba8();
    let size = [rgba.width(), rgba.height()];
    let dimensions_match = size == expected_size || size == [expected_size[1], expected_size[0]];
    if !dimensions_match || rgba.as_raw().len() != checked_rgba_bytes(size)? {
        return Err(CopyFrameError::Decode(format!(
            "decoded dimensions {} x {} do not match source dimensions {} x {}",
            size[0], size[1], expected_size[0], expected_size[1]
        )));
    }
    Ok(CopyFrameImage {
        size,
        rgba: rgba.into_raw(),
    })
}

fn checked_rgba_bytes(size: [u32; 2]) -> Result<usize, CopyFrameError> {
    let bytes = usize::try_from(size[0])
        .ok()
        .and_then(|width| {
            usize::try_from(size[1])
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| size[0] > 0 && size[1] > 0 && *bytes <= COPY_FRAME_MAX_RGBA_BYTES)
        .ok_or(CopyFrameError::FrameTooLarge {
            size,
            maximum_bytes: COPY_FRAME_MAX_RGBA_BYTES,
        })?;
    Ok(bytes)
}

#[cfg(windows)]
fn dibv5_payload_size(image: &CopyFrameImage) -> Result<usize, CopyFrameError> {
    std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPV5HEADER>()
        .checked_add(checked_rgba_bytes(image.size)?)
        .ok_or_else(|| CopyFrameError::Clipboard("clipboard allocation overflowed".to_owned()))
}

#[cfg(windows)]
fn write_dibv5_payload(
    image: &CopyFrameImage,
    destination: &mut [u8],
) -> Result<(), CopyFrameError> {
    use std::{mem::size_of, ptr};

    use windows::Win32::{
        Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER, LCS_GM_IMAGES},
        UI::ColorSystem::LCS_sRGB,
    };

    let [width, height] = image.size;
    let pixel_bytes = checked_rgba_bytes(image.size)?;
    if destination.len() != dibv5_payload_size(image)? {
        return Err(CopyFrameError::Clipboard(
            "clipboard image allocation has an unexpected size".to_owned(),
        ));
    }
    let width = i32::try_from(width).map_err(|_| {
        CopyFrameError::Clipboard("frame width exceeds Windows bitmap limits".to_owned())
    })?;
    let height = i32::try_from(height).map_err(|_| {
        CopyFrameError::Clipboard("frame height exceeds Windows bitmap limits".to_owned())
    })?;
    let size_image = u32::try_from(pixel_bytes).map_err(|_| {
        CopyFrameError::Clipboard("frame pixels exceed Windows bitmap limits".to_owned())
    })?;
    let header = BITMAPV5HEADER {
        bV5Size: size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: width,
        bV5Height: -height,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: size_image,
        bV5RedMask: 0x00ff_0000,
        bV5GreenMask: 0x0000_ff00,
        bV5BlueMask: 0x0000_00ff,
        bV5AlphaMask: 0xff00_0000,
        bV5CSType: LCS_sRGB.0 as u32,
        bV5Intent: LCS_GM_IMAGES as u32,
        ..Default::default()
    };

    unsafe {
        ptr::copy_nonoverlapping(
            (&header as *const BITMAPV5HEADER).cast::<u8>(),
            destination.as_mut_ptr(),
            size_of::<BITMAPV5HEADER>(),
        );
        let pixels = destination.as_mut_ptr().add(size_of::<BITMAPV5HEADER>());
        for (index, rgba) in image.rgba.chunks_exact(4).enumerate() {
            let bgra = pixels.add(index * 4);
            *bgra = rgba[2];
            *bgra.add(1) = rgba[1];
            *bgra.add(2) = rgba[0];
            *bgra.add(3) = rgba[3];
        }
    }
    Ok(())
}

#[cfg(windows)]
fn publish_image_to_clipboard(
    image: &CopyFrameImage,
    native_window: Option<isize>,
) -> Result<(), CopyFrameError> {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HWND},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
    };

    const CF_DIBV5: u32 = 17;

    struct ClipboardGuard;
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    let native_window = native_window.ok_or_else(|| {
        CopyFrameError::Clipboard("Apollo's native window handle is unavailable".to_owned())
    })?;
    let total_bytes = dibv5_payload_size(image)?;

    unsafe {
        OpenClipboard(Some(HWND(native_window as *mut _))).map_err(|error| {
            CopyFrameError::Clipboard(format!("could not open clipboard: {error}"))
        })?;
        let _guard = ClipboardGuard;
        EmptyClipboard().map_err(|error| {
            CopyFrameError::Clipboard(format!("could not clear clipboard: {error}"))
        })?;
        let memory = GlobalAlloc(GMEM_MOVEABLE, total_bytes).map_err(|error| {
            CopyFrameError::Clipboard(format!("could not allocate clipboard image: {error}"))
        })?;
        let destination = GlobalLock(memory).cast::<u8>();
        if destination.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(CopyFrameError::Clipboard(
                "could not lock clipboard image memory".to_owned(),
            ));
        }
        let destination = std::slice::from_raw_parts_mut(destination, total_bytes);
        if let Err(error) = write_dibv5_payload(image, destination) {
            let _ = GlobalUnlock(memory);
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        let _ = GlobalUnlock(memory);
        if let Err(error) = SetClipboardData(CF_DIBV5, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(CopyFrameError::Clipboard(format!(
                "could not publish clipboard image: {error}"
            )));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_image_to_clipboard(
    _image: &CopyFrameImage,
    _native_window: Option<isize>,
) -> Result<(), CopyFrameError> {
    Err(CopyFrameError::Clipboard(
        "image clipboard publication is supported only on Windows".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Cursor,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use image::{DynamicImage, ImageFormat, RgbaImage};

    use crate::{
        frame_capture::{
            command::build_copy_frame_invocation,
            model::{FrameTimeBase, FrameTimestamp, ResolvedFramePosition},
        },
        media::{
            error::MediaErrorKind,
            execution::ProcessControl,
            intake::SourceIdentity,
            model::{MediaFile, VideoStream},
            process::{MediaProcessFailure, MediaProcessState, MediaProcessWake},
            tools::MediaTools,
        },
    };

    use super::{
        COPY_FRAME_MAX_RGBA_BYTES, CopyFrameError, CopyFrameExtraction, CopyFrameExtractionOwner,
        CopyFrameProcessControl, CopyFrameRequest, CopyFrameWorker, checked_rgba_bytes,
        copy_frame_action_eligible, decode_copy_frame_png,
    };
    #[cfg(windows)]
    use super::{CopyFrameImage, dibv5_payload_size, write_dibv5_payload};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn media(path: PathBuf, size: [u32; 2]) -> MediaFile {
        MediaFile {
            path,
            duration: Some(std::time::Duration::from_secs(30)),
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 7,
                codec: Some("h264".to_owned()),
                width: Some(size[0]),
                height: Some(size[1]),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: Vec::new(),
        }
    }

    fn request(size: [u32; 2]) -> (PathBuf, CopyFrameRequest) {
        let directory = std::env::temp_dir().join(format!(
            "apollo-copy-frame-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("source.mp4");
        fs::write(&path, b"source").unwrap();
        let identity = SourceIdentity::read(&path).unwrap();
        let request = CopyFrameRequest::new(
            &identity,
            &media(path, size),
            FrameTimestamp::from_milliseconds(1_250).into(),
        )
        .unwrap();
        (directory, request)
    }

    fn png_bytes(size: [u32; 2]) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(RgbaImage::new(size[0], size[1]))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn arguments(request: &CopyFrameRequest) -> Vec<String> {
        build_copy_frame_invocation(request)
            .args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn command_copies_one_exact_primary_frame_without_scaling_or_public_output() {
        let (directory, mut request) = request([1920, 1080]);
        request.position = ResolvedFramePosition::new(1001, FrameTimeBase::new(1, 24_000).unwrap())
            .unwrap()
            .into();
        let args = arguments(&request);

        assert!(args.windows(2).any(|pair| pair == ["-map", "0:7"]));
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "1"]));
        assert!(args.windows(2).any(|pair| pair == ["-an", "-c:v"]));
        assert!(args.windows(2).any(|pair| pair == ["-f", "image2pipe"]));
        assert_eq!(args.last().map(String::as_str), Some("pipe:1"));
        assert!(!args.iter().any(|argument| argument == "-vf"));
        assert_eq!(args[1], request.position.ffmpeg_timestamp());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn checked_copy_allocation_accepts_4k_and_8k_and_rejects_unsafe_dimensions() {
        assert_eq!(checked_rgba_bytes([3840, 2160]).unwrap(), 33_177_600);
        assert_eq!(checked_rgba_bytes([7680, 4320]).unwrap(), 132_710_400);
        assert!(matches!(
            checked_rgba_bytes([16_384, 16_384]),
            Err(CopyFrameError::FrameTooLarge {
                maximum_bytes: COPY_FRAME_MAX_RGBA_BYTES,
                ..
            })
        ));
        assert!(checked_rgba_bytes([0, 1080]).is_err());
    }

    #[test]
    fn action_eligibility_defers_oversized_allocation_failure_to_request_preflight() {
        let (directory, normal_request) = request([1920, 1080]);
        let position = normal_request.position;
        let normal_media = media(normal_request.input.clone(), [1920, 1080]);
        assert!(copy_frame_action_eligible(&normal_media, position));

        let invalid_position = FrameTimestamp::from_milliseconds(30_000).into();
        assert!(!copy_frame_action_eligible(&normal_media, invalid_position));
        let mut missing_video = normal_media.clone();
        missing_video.video_streams.clear();
        assert!(!copy_frame_action_eligible(&missing_video, position));

        let oversized_media = media(normal_request.input.clone(), [16_384, 16_384]);
        assert!(copy_frame_action_eligible(&oversized_media, position));
        let error =
            CopyFrameRequest::new(&normal_request.source_identity, &oversized_media, position)
                .unwrap_err();
        assert!(matches!(error, CopyFrameError::FrameTooLarge { .. }));
        assert_eq!(
            error.user_error().kind(),
            MediaErrorKind::FrameTooLargeToCopy
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn decode_requires_the_full_expected_dimensions_without_downscaling() {
        let image = decode_copy_frame_png(&png_bytes([4, 2]), [4, 2]).unwrap();
        assert_eq!(image.size(), [4, 2]);
        assert_eq!(image.rgba().len(), 4 * 2 * 4);
        assert!(decode_copy_frame_png(&png_bytes([2, 1]), [4, 2]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_clipboard_payload_is_top_down_dibv5_with_bgra_pixels() {
        use std::mem::size_of;

        use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER};

        let image = CopyFrameImage {
            size: [2, 1],
            rgba: vec![0x11, 0x22, 0x33, 0x44, 0xaa, 0xbb, 0xcc, 0xdd],
        };
        let mut payload = vec![0; dibv5_payload_size(&image).unwrap()];
        write_dibv5_payload(&image, &mut payload).unwrap();

        let header = unsafe { payload.as_ptr().cast::<BITMAPV5HEADER>().read_unaligned() };
        assert_eq!(header.bV5Size as usize, size_of::<BITMAPV5HEADER>());
        assert_eq!([header.bV5Width, header.bV5Height], [2, -1]);
        assert_eq!(header.bV5BitCount, 32);
        assert_eq!(header.bV5Compression, BI_BITFIELDS);
        assert_eq!(header.bV5SizeImage, 8);
        assert_eq!(header.bV5AlphaMask, 0xff00_0000);
        assert_eq!(
            &payload[size_of::<BITMAPV5HEADER>()..],
            &[0x33, 0x22, 0x11, 0x44, 0xcc, 0xbb, 0xaa, 0xdd]
        );
    }

    #[test]
    fn background_worker_keeps_the_original_request_and_runs_publication_work() {
        let (directory, request) = request([4, 2]);
        let observed = Arc::new(Mutex::new(None));
        let worker = CopyFrameWorker::spawn_with(request.clone(), || {}, {
            let observed = Arc::clone(&observed);
            move |request| {
                *observed.lock().unwrap() =
                    Some((request.source_identity.clone(), request.position));
                Ok(())
            }
        })
        .unwrap();

        let result = loop {
            if let Some(result) = worker.poll() {
                break result;
            }
            std::thread::yield_now();
        };
        assert_eq!(result, Ok(()));
        assert_eq!(
            observed.lock().unwrap().as_ref(),
            Some(&(request.source_identity, request.position))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extraction_returns_exact_png_bytes_with_the_original_request() {
        let (directory, request) = request([4, 2]);
        let payload = png_bytes([4, 2]);
        let mut extraction = CopyFrameExtraction::start_with_launcher(
            &MediaTools::new("resolved-ffmpeg", "resolved-ffprobe"),
            request.clone(),
            |_| Ok(FakeProcess::completed(payload.clone())),
        )
        .unwrap();

        let completion = extraction.poll().unwrap();
        assert_eq!(completion.request, request);
        assert_eq!(completion.result, Ok(payload));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extraction_failure_retains_diagnostic_and_active_owner_rejects_competition() {
        let (directory, request) = request([4, 2]);
        let mut failed = CopyFrameExtraction::start_with_launcher(
            &MediaTools::new("resolved-ffmpeg", "resolved-ffprobe"),
            request.clone(),
            |_| {
                Ok(FakeProcess::failed(
                    MediaProcessFailure::Exit(Some(9)),
                    "decoder failed",
                ))
            },
        )
        .unwrap();
        assert!(matches!(
            failed.poll().unwrap().result,
            Err(CopyFrameError::Process { diagnostic, .. }) if diagnostic == "decoder failed"
        ));

        let process = FakeProcess::running();
        let process_state = Arc::clone(&process.state);
        let active = CopyFrameExtraction::start_with_launcher(
            &MediaTools::new("resolved-ffmpeg", "resolved-ffprobe"),
            request.clone(),
            |_| Ok(process),
        )
        .unwrap();
        let mut owner = CopyFrameExtractionOwner {
            active: Some(active),
        };
        assert!(matches!(
            owner.start(
                &MediaTools::new("other-ffmpeg", "other-ffprobe"),
                request,
                MediaProcessWake::default(),
            ),
            Err(CopyFrameError::AlreadyRunning)
        ));
        assert!(owner.cancel());
        assert_eq!(*process_state.lock().unwrap(), MediaProcessState::Cancelled);
        fs::remove_dir_all(directory).unwrap();
    }

    struct FakeProcess {
        state: Arc<Mutex<MediaProcessState>>,
        stdout: Vec<u8>,
        diagnostic: String,
    }

    impl FakeProcess {
        fn running() -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
                stdout: Vec::new(),
                diagnostic: String::new(),
            }
        }

        fn completed(stdout: Vec<u8>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Completed)),
                stdout,
                diagnostic: String::new(),
            }
        }

        fn failed(reason: MediaProcessFailure, diagnostic: &str) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Failed(reason))),
                stdout: Vec::new(),
                diagnostic: diagnostic.to_owned(),
            }
        }
    }

    impl ProcessControl for FakeProcess {
        fn state(&self) -> MediaProcessState {
            self.state.lock().unwrap().clone()
        }

        fn cancel(&self) -> bool {
            let mut state = self.state.lock().unwrap();
            if *state != MediaProcessState::Running {
                return false;
            }
            *state = MediaProcessState::Cancelled;
            true
        }

        fn diagnostic_stderr(&self) -> String {
            self.diagnostic.clone()
        }

        fn drain_progress(&self) -> Vec<String> {
            Vec::new()
        }
    }

    impl CopyFrameProcessControl for FakeProcess {
        fn captured_stdout_bytes(&self) -> Vec<u8> {
            self.stdout.clone()
        }
    }
}
