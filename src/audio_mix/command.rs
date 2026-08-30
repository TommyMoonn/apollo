use std::{ffi::OsString, fmt};

use crate::media::model::MediaFile;

use super::model::{
    AudioMixRequest, LoudnessNormalization, LoudnessTargets, OutputContainer, SelectedAudioTrack,
};

const MIXED_AUDIO_LABEL: &str = "mixed";
const PRE_LIMIT_LABEL: &str = "pre_limit";
const PRE_NORMALIZATION_LABEL: &str = "pre_norm";

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

    if request.selected_tracks.is_empty() {
        return Err(AudioMixCommandError::NoAudioStreamsSelected);
    }

    let mut args = vec![
        OsString::from("-i"),
        request.input.as_os_str().to_os_string(),
        OsString::from("-filter_complex"),
        OsString::from(mix_filter(&request.selected_tracks, request.normalization)),
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
        OsString::from("-progress"),
        OsString::from("pipe:1"),
        OsString::from("-nostats"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        request.output.as_os_str().to_os_string(),
    ]);

    Ok(FfmpegInvocation { args })
}

fn mix_filter(
    selected_tracks: &[SelectedAudioTrack],
    normalization: LoudnessNormalization,
) -> String {
    let volume_stages = selected_tracks
        .iter()
        .enumerate()
        .map(|(position, track)| {
            format!(
                "[0:{}]volume={}[mix_{position}]",
                track.stream_index, track.gain
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let mix_inputs = selected_tracks
        .iter()
        .enumerate()
        .map(|(position, _)| format!("[mix_{position}]"))
        .collect::<String>();

    let limiter_output = match normalization {
        LoudnessNormalization::Off => MIXED_AUDIO_LABEL,
        LoudnessNormalization::Enabled(_) => PRE_NORMALIZATION_LABEL,
    };
    let mut filter = format!(
        "{volume_stages};{mix_inputs}amix=inputs={}:duration=longest:normalize=0[{PRE_LIMIT_LABEL}];[{PRE_LIMIT_LABEL}]alimiter=limit=0.95:level=0:latency=1[{limiter_output}]",
        selected_tracks.len(),
    );

    if let LoudnessNormalization::Enabled(targets) = normalization {
        filter.push_str(&loudness_normalization_stage(targets));
    }

    filter
}

fn loudness_normalization_stage(targets: LoudnessTargets) -> String {
    format!(
        ";[{PRE_NORMALIZATION_LABEL}]loudnorm=I={}:TP={}:LRA={}[{MIXED_AUDIO_LABEL}]",
        decimal_tenths(targets.integrated_loudness_tenths()),
        decimal_tenths(targets.maximum_true_peak_tenths()),
        decimal_tenths(targets.loudness_range_tenths()),
    )
}

fn decimal_tenths(value: i16) -> String {
    let magnitude = i32::from(value).abs();
    let sign = if value < 0 { "-" } else { "" };
    format!("{sign}{}.{:01}", magnitude / 10, magnitude % 10)
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
