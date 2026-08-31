use std::ffi::OsString;

use super::model::{FrameCaptureFormat, FrameCaptureRequest};

const JPEG_QUALITY: &str = "2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub args: Vec<OsString>,
}

/// Build structured FFmpeg arguments from an already validated Frame Capture request.
pub fn build_frame_capture_invocation(
    request: &FrameCaptureRequest,
    primary_video_stream_index: u32,
) -> FfmpegInvocation {
    let mut args = vec![
        OsString::from("-ss"),
        OsString::from(request.timestamp.to_string()),
        OsString::from("-i"),
        request.input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from(format!("0:{primary_video_stream_index}")),
        OsString::from("-frames:v"),
        OsString::from("1"),
        OsString::from("-abort_on"),
        OsString::from("empty_output"),
        OsString::from("-an"),
        OsString::from("-c:v"),
        OsString::from(match request.format {
            FrameCaptureFormat::Png => "png",
            FrameCaptureFormat::Jpeg => "mjpeg",
        }),
    ];

    if request.format == FrameCaptureFormat::Jpeg {
        args.extend([OsString::from("-q:v"), OsString::from(JPEG_QUALITY)]);
    }

    args.extend([
        OsString::from("-progress"),
        OsString::from("pipe:1"),
        OsString::from("-nostats"),
        OsString::from("-f"),
        OsString::from(request.format.ffmpeg_muxer_name()),
        request.output.as_os_str().to_os_string(),
    ]);
    FfmpegInvocation { args }
}
