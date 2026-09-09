use std::{ffi::OsString, fmt};

use crate::media::{
    container::MediaContainer,
    model::{MediaFile, StreamKind},
};

use super::{
    model::{EncoderQuality, UpscaleRequest, VideoCodec, VideoEncoderBackend},
    sizing::OutputDimensions,
    validation::{UpscaleValidationError, resolve_output_pixel_format},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub args: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpscaleCommandError {
    MissingVideoStream,
    NvencUnavailable { reason: String },
    VideoFormat(UpscaleValidationError),
}

impl fmt::Display for UpscaleCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VideoFormat(error) => write!(f, "{error}"),
            Self::MissingVideoStream => write!(f, "upscale command requires a video stream"),
            Self::NvencUnavailable { reason } => {
                write!(f, "NVIDIA NVENC is unavailable: {reason}")
            }
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
        .ok_or(UpscaleCommandError::MissingVideoStream)?;
    let resolved_encoder =
        request
            .resolve_video_encoder()
            .map_err(|error| UpscaleCommandError::NvencUnavailable {
                reason: error.reason,
            })?;
    let video_encoder = match (resolved_encoder.backend, request.codec) {
        (VideoEncoderBackend::Cpu, VideoCodec::H264) => "libx264",
        (VideoEncoderBackend::Cpu, VideoCodec::H265) => "libx265",
        (VideoEncoderBackend::NvidiaNvenc, VideoCodec::H264) => "h264_nvenc",
        (VideoEncoderBackend::NvidiaNvenc, VideoCodec::H265) => "hevc_nvenc",
    };
    let pixel_format = resolve_output_pixel_format(
        &primary_video_stream.format,
        request.codec,
        resolved_encoder.backend,
    )
    .map_err(UpscaleCommandError::VideoFormat)?;
    let stream_index = primary_video_stream.stream_index;

    let mut args = vec![
        OsString::from("-i"),
        request.input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from(format!("0:{stream_index}")),
        OsString::from("-map"),
        OsString::from("0:a?"),
        OsString::from("-vf"),
        OsString::from(format!(
            "scale={}:{}:flags=lanczos",
            dimensions.width, dimensions.height
        )),
        OsString::from("-c:v"),
        OsString::from(video_encoder),
    ];

    args.extend([OsString::from("-pix_fmt:v:0"), OsString::from(pixel_format)]);
    let format = &primary_video_stream.format;
    for (option, value) in [
        ("-color_range:v:0", &format.color_range),
        ("-colorspace:v:0", &format.color_space),
        ("-color_trc:v:0", &format.color_transfer),
        ("-color_primaries:v:0", &format.color_primaries),
    ] {
        if let Some(value) = value {
            args.extend([OsString::from(option), OsString::from(value)]);
        }
    }

    match resolved_encoder.quality {
        EncoderQuality::Cpu(quality) => args.extend([
            OsString::from("-crf"),
            OsString::from(quality.value().to_string()),
            OsString::from("-preset"),
            OsString::from("slow"),
        ]),
        EncoderQuality::NvidiaNvenc(quality) => args.extend([
            OsString::from("-preset"),
            OsString::from("p5"),
            OsString::from("-rc"),
            OsString::from("vbr"),
            OsString::from("-cq"),
            OsString::from(quality.value().to_string()),
        ]),
    }

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
