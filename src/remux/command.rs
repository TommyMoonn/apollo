use std::ffi::OsString;

use super::model::RemuxRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub args: Vec<OsString>,
}

/// Build structured FFmpeg arguments from an already validated Remux request.
pub fn build_remux_invocation(request: &RemuxRequest) -> FfmpegInvocation {
    FfmpegInvocation {
        args: vec![
            OsString::from("-i"),
            request.input.as_os_str().to_os_string(),
            OsString::from("-map"),
            OsString::from("0"),
            OsString::from("-c"),
            OsString::from("copy"),
            OsString::from("-map_metadata"),
            OsString::from("0"),
            OsString::from("-map_chapters"),
            OsString::from("0"),
            OsString::from("-progress"),
            OsString::from("pipe:1"),
            OsString::from("-nostats"),
            OsString::from("-f"),
            OsString::from(request.container.ffmpeg_muxer_name()),
            request.output.as_os_str().to_os_string(),
        ],
    }
}
