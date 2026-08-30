use std::{ffi::OsString, fmt};

use crate::media::{
    container::MediaContainer,
    model::{MediaFile, StreamKind},
};

use super::{
    model::{UpscaleRequest, VideoCodec},
    sizing::OutputDimensions,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub args: Vec<OsString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpscaleCommandError {
    MissingVideoStream,
}

impl fmt::Display for UpscaleCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => write!(f, "upscale command requires a video stream"),
        }
    }
}

impl std::error::Error for UpscaleCommandError {}

/// Build structured FFmpeg arguments from an already validated Upscale request.
///
/// Output sizing remains owned by `sizing.rs`; this builder only consumes the
/// precomputed dimensions and probe metadata needed for stream mapping and the
/// fixed MP4 audio compatibility policy.
pub fn build_upscale_invocation(
    request: &UpscaleRequest,
    media: &MediaFile,
    dimensions: OutputDimensions,
) -> Result<FfmpegInvocation, UpscaleCommandError> {
    let primary_video_stream = media
        .video_streams
        .first()
        .ok_or(UpscaleCommandError::MissingVideoStream)?
        .stream_index;
    let (video_encoder, crf) = match request.codec {
        VideoCodec::H264 => ("libx264", "18"),
        VideoCodec::H265 => ("libx265", "20"),
    };

    let mut args = vec![
        OsString::from("-i"),
        request.input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from(format!("0:{primary_video_stream}")),
        OsString::from("-map"),
        OsString::from("0:a?"),
        OsString::from("-vf"),
        OsString::from(format!(
            "scale={}:{}:flags=lanczos",
            dimensions.width, dimensions.height
        )),
        OsString::from("-c:v"),
        OsString::from(video_encoder),
        OsString::from("-crf"),
        OsString::from(crf),
        OsString::from("-preset"),
        OsString::from("slow"),
    ];

    if let Some(audio_codec) = mp4_audio_codec(media) {
        args.extend([OsString::from("-c:a"), OsString::from(audio_codec)]);
    }

    args.extend([
        OsString::from("-progress"),
        OsString::from("pipe:1"),
        OsString::from("-nostats"),
        OsString::from("-f"),
        OsString::from("mp4"),
        request.output.as_os_str().to_os_string(),
    ]);

    Ok(FfmpegInvocation { args })
}

fn mp4_audio_codec(media: &MediaFile) -> Option<&'static str> {
    if media.audio_streams.is_empty() {
        return None;
    }

    if media.audio_streams.iter().all(|stream| {
        MediaContainer::Mp4
            .is_stream_copy_codec_compatible(StreamKind::Audio, stream.codec.as_deref())
    }) {
        Some("copy")
    } else {
        Some("aac")
    }
}
