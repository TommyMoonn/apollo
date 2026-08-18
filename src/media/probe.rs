use std::{
    fmt, io,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    time::Duration,
};

use serde::Deserialize;

use super::{
    model::{AudioStream, FrameRate, MediaFile, VideoStream},
    tools::MediaTools,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaProbeError {
    LaunchFailed(io::ErrorKind),
    FfprobeFailed(Option<i32>),
    InvalidOutput {
        message: String,
        line: usize,
        column: usize,
    },
}

impl fmt::Display for MediaProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LaunchFailed(kind) => write!(f, "FFprobe could not start ({kind:?})"),
            Self::FfprobeFailed(Some(code)) => {
                write!(f, "FFprobe exited with status code {code}")
            }
            Self::FfprobeFailed(None) => write!(f, "FFprobe exited unsuccessfully"),
            Self::InvalidOutput {
                message,
                line,
                column,
            } => write!(
                f,
                "FFprobe returned invalid JSON at line {line}, column {column}: {message}"
            ),
        }
    }
}

impl std::error::Error for MediaProbeError {}

#[derive(Debug, Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Debug, Deserialize)]
struct RawFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStream {
    index: u32,
    codec_type: Option<String>,
    codec_name: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    tags: Option<RawTags>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTags {
    title: Option<String>,
    language: Option<String>,
}

pub fn probe_media(tools: &MediaTools, path: &Path) -> Result<MediaFile, MediaProbeError> {
    let output = ffprobe_command(&tools.ffprobe, path)
        .output()
        .map_err(|error| MediaProbeError::LaunchFailed(error.kind()))?;

    validate_exit_status(output.status)?;
    parse_ffprobe_json(path, &output.stdout)
}

pub fn parse_ffprobe_json(path: &Path, json: &[u8]) -> Result<MediaFile, MediaProbeError> {
    let raw: RawProbe =
        serde_json::from_slice(json).map_err(|error| MediaProbeError::InvalidOutput {
            message: error.to_string(),
            line: error.line(),
            column: error.column(),
        })?;

    Ok(normalize_probe(path, raw))
}

fn ffprobe_command(executable: &Path, input: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn validate_exit_status(status: ExitStatus) -> Result<(), MediaProbeError> {
    if status.success() {
        Ok(())
    } else {
        Err(MediaProbeError::FfprobeFailed(status.code()))
    }
}

fn normalize_probe(path: &Path, raw: RawProbe) -> MediaFile {
    let duration = raw
        .format
        .and_then(|format| format.duration)
        .and_then(|duration| parse_duration(&duration));

    let mut video_streams = Vec::new();
    let mut audio_streams = Vec::new();

    for stream in raw.streams {
        match stream.codec_type.as_deref() {
            Some("video") => video_streams.push(VideoStream {
                stream_index: stream.index,
                codec: nonempty(stream.codec_name),
                width: stream.width,
                height: stream.height,
                frame_rate: stream
                    .avg_frame_rate
                    .as_deref()
                    .and_then(parse_frame_rate)
                    .or_else(|| stream.r_frame_rate.as_deref().and_then(parse_frame_rate)),
            }),
            Some("audio") => {
                let tags = stream.tags.unwrap_or_default();
                audio_streams.push(AudioStream {
                    stream_index: stream.index,
                    codec: nonempty(stream.codec_name),
                    channels: stream.channels,
                    channel_layout: nonempty(stream.channel_layout),
                    title: nonempty(tags.title),
                    language: nonempty(tags.language),
                });
            }
            _ => {}
        }
    }

    MediaFile {
        path: path.to_path_buf(),
        duration,
        video_streams,
        audio_streams,
    }
}

fn parse_duration(value: &str) -> Option<Duration> {
    let seconds = value.parse::<f64>().ok()?;
    Duration::try_from_secs_f64(seconds).ok()
}

fn parse_frame_rate(value: &str) -> Option<FrameRate> {
    let (numerator, denominator) = value.split_once('/')?;
    FrameRate::new(numerator.parse().ok()?, denominator.parse().ok()?)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, path::Path};

    use super::{ffprobe_command, parse_frame_rate};
    use crate::media::model::FrameRate;

    #[test]
    fn ffprobe_command_requests_json_format_and_stream_metadata() {
        let command = ffprobe_command(Path::new("/tools/ffprobe"), Path::new("input file.mkv"));
        let args = command.get_args().collect::<Vec<_>>();

        assert_eq!(command.get_program(), OsStr::new("/tools/ffprobe"));
        assert_eq!(
            args,
            [
                OsStr::new("-v"),
                OsStr::new("error"),
                OsStr::new("-show_format"),
                OsStr::new("-show_streams"),
                OsStr::new("-of"),
                OsStr::new("json"),
                OsStr::new("input file.mkv"),
            ]
        );
    }

    #[test]
    fn invalid_or_zero_frame_rates_are_absent() {
        assert_eq!(
            parse_frame_rate("30000/1001"),
            FrameRate::new(30_000, 1_001)
        );
        assert_eq!(parse_frame_rate("0/0"), None);
        assert_eq!(parse_frame_rate("N/A"), None);
    }
}
