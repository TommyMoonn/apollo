use std::{ffi::OsString, fmt};

use crate::media::model::MediaFile;

use super::model::{AudioMixRequest, OutputContainer};

const MIXED_AUDIO_LABEL: &str = "mixed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub args: Vec<OsString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMixCommandError {
    MissingVideoStream,
    NoAudioStreamsSelected,
}

impl fmt::Display for AudioMixCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => write!(f, "audio mix command requires a video stream"),
            Self::NoAudioStreamsSelected => {
                write!(
                    f,
                    "audio mix command requires at least one selected audio stream"
                )
            }
        }
    }
}

impl std::error::Error for AudioMixCommandError {}

/// Build the structured FFmpeg arguments for an already validated Audio Mix request.
///
/// Stream-existence and duplicate-selection rules remain owned by request validation. This
/// builder only guards the structural inputs required to create a valid filter graph and primary
/// video mapping.
pub fn build_audio_mix_invocation(
    request: &AudioMixRequest,
    media: &MediaFile,
) -> Result<FfmpegInvocation, AudioMixCommandError> {
    let primary_video_stream = media
        .video_streams
        .first()
        .ok_or(AudioMixCommandError::MissingVideoStream)?
        .stream_index;

    if request.selected_streams.is_empty() {
        return Err(AudioMixCommandError::NoAudioStreamsSelected);
    }

    let mut args = vec![
        OsString::from("-i"),
        request.input.as_os_str().to_os_string(),
        OsString::from("-filter_complex"),
        OsString::from(mix_filter(&request.selected_streams)),
    ];

    match request.container {
        OutputContainer::Mp4 => append_mp4_mapping(&mut args, primary_video_stream),
        OutputContainer::Mkv => append_mkv_mapping(&mut args),
    }

    args.extend([
        OsString::from("-map_metadata"),
        OsString::from("0"),
        OsString::from("-map_chapters"),
        OsString::from("0"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        request.output.as_os_str().to_os_string(),
    ]);

    Ok(FfmpegInvocation { args })
}

fn mix_filter(selected_streams: &[u32]) -> String {
    let inputs = selected_streams
        .iter()
        .map(|stream_index| format!("[0:{stream_index}]"))
        .collect::<String>();

    format!(
        "{inputs}amix=inputs={}:duration=longest[{MIXED_AUDIO_LABEL}]",
        selected_streams.len()
    )
}

fn append_mp4_mapping(args: &mut Vec<OsString>, primary_video_stream: u32) {
    args.extend([
        OsString::from("-map"),
        OsString::from(format!("0:{primary_video_stream}")),
        OsString::from("-map"),
        OsString::from(format!("[{MIXED_AUDIO_LABEL}]")),
        OsString::from("-c:v"),
        OsString::from("copy"),
    ]);
}

fn append_mkv_mapping(args: &mut Vec<OsString>) {
    args.extend([
        OsString::from("-map"),
        OsString::from("0"),
        OsString::from("-map"),
        OsString::from("-0:a"),
        OsString::from("-map"),
        OsString::from("-0:t?"),
        OsString::from("-map"),
        OsString::from(format!("[{MIXED_AUDIO_LABEL}]")),
        OsString::from("-map"),
        OsString::from("0:t?"),
        OsString::from("-c"),
        OsString::from("copy"),
        OsString::from("-c:v"),
        OsString::from("copy"),
    ]);
}
