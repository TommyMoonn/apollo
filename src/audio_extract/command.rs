use std::ffi::OsString;

use super::model::{AudioExtractFormat, AudioExtractMetadata, AudioExtractRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub args: Vec<OsString>,
}

/// Build structured FFmpeg arguments from an already validated Audio Extract request.
pub fn build_audio_extract_invocation(
    request: &AudioExtractRequest,
    metadata: &AudioExtractMetadata,
) -> FfmpegInvocation {
    let mut args = vec![
        OsString::from("-i"),
        request.input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from(format!("0:{}", request.audio_stream_index)),
        OsString::from("-c:a"),
        OsString::from(match request.format {
            AudioExtractFormat::Original => "copy",
            AudioExtractFormat::Aac => "aac",
            AudioExtractFormat::Flac => "flac",
        }),
    ];

    if request.format == AudioExtractFormat::Aac {
        args.extend([OsString::from("-b:a"), OsString::from("256k")]);
    }

    args.extend([OsString::from("-map_metadata"), OsString::from("-1")]);
    append_selected_track_metadata(&mut args, request.format, metadata);

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

fn append_selected_track_metadata(
    args: &mut Vec<OsString>,
    format: AudioExtractFormat,
    metadata: &AudioExtractMetadata,
) {
    match format {
        AudioExtractFormat::Original => {
            append_metadata(args, "-metadata:s:a:0", "title", metadata.title.as_deref());
            append_metadata(
                args,
                "-metadata:s:a:0",
                "language",
                metadata.language.as_deref(),
            );
        }
        AudioExtractFormat::Aac => {
            append_metadata(args, "-metadata", "title", metadata.title.as_deref());
            append_metadata(
                args,
                "-metadata:s:a:0",
                "language",
                metadata.language.as_deref(),
            );
        }
        AudioExtractFormat::Flac => {
            append_metadata(args, "-metadata", "title", metadata.title.as_deref());
            append_metadata(args, "-metadata", "language", metadata.language.as_deref());
        }
    }
}

fn append_metadata(args: &mut Vec<OsString>, option: &str, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        args.extend([
            OsString::from(option),
            OsString::from(format!("{key}={value}")),
        ]);
    }
}
