use std::{ffi::OsString, path::Path};

use super::{
    clipboard::CopyFrameRequest,
    model::{FrameCaptureFormat, FrameCaptureRequest, FramePosition},
    preview::{PREVIEW_MAXIMUM_EDGE, PreviewRequest},
};

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
    let mut args =
        frame_selection_args(&request.input, primary_video_stream_index, request.position);
    args.extend([
        OsString::from("-c:v"),
        OsString::from(match request.format {
            FrameCaptureFormat::Png => "png",
            FrameCaptureFormat::Jpeg => "mjpeg",
        }),
    ]);

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

pub fn build_preview_invocation(request: &PreviewRequest) -> FfmpegInvocation {
    let mut args =
        frame_selection_args(&request.input, request.video_stream_index, request.position);
    args.extend([
        OsString::from("-vf"),
        OsString::from(format!(
            "scale=w='min({PREVIEW_MAXIMUM_EDGE},iw)':h='min({PREVIEW_MAXIMUM_EDGE},ih)':force_original_aspect_ratio=decrease"
        )),
        OsString::from("-pix_fmt"),
        OsString::from("rgba"),
        OsString::from("-c:v"),
        OsString::from("png"),
        OsString::from("-nostats"),
        OsString::from("-f"),
        OsString::from("image2pipe"),
        OsString::from("pipe:1"),
    ]);
    FfmpegInvocation { args }
}

pub fn build_copy_frame_invocation(request: &CopyFrameRequest) -> FfmpegInvocation {
    let mut args =
        frame_selection_args(&request.input, request.video_stream_index, request.position);
    args.extend([
        OsString::from("-c:v"),
        OsString::from("png"),
        OsString::from("-nostats"),
        OsString::from("-f"),
        OsString::from("image2pipe"),
        OsString::from("pipe:1"),
    ]);
    FfmpegInvocation { args }
}

fn frame_selection_args(
    input: &Path,
    primary_video_stream_index: u32,
    position: FramePosition,
) -> Vec<OsString> {
    vec![
        OsString::from("-ss"),
        OsString::from(position.ffmpeg_timestamp()),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from(format!("0:{primary_video_stream_index}")),
        OsString::from("-frames:v"),
        OsString::from("1"),
        OsString::from("-abort_on"),
        OsString::from("empty_output"),
        OsString::from("-an"),
    ]
}
