use std::{ffi::OsString, path::PathBuf, str::FromStr, time::Duration};

use apollo::{
    frame_capture::{
        command::build_frame_capture_invocation,
        model::{FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp},
        validation::{FrameCaptureValidationError, validate_frame_capture_request},
    },
    media::model::{MediaFile, MediaStream, StreamKind, VideoStream},
};

fn media(duration: Option<Duration>) -> MediaFile {
    MediaFile {
        path: PathBuf::from("source/input.mkv"),
        duration,
        streams: vec![
            MediaStream {
                stream_index: 2,
                kind: StreamKind::Audio,
                codec: Some("aac".into()),
            },
            MediaStream {
                stream_index: 7,
                kind: StreamKind::Video,
                codec: Some("h264".into()),
            },
        ],
        video_streams: vec![VideoStream {
            stream_index: 7,
            codec: Some("h264".into()),
            width: Some(1920),
            height: Some(1080),
            frame_rate: None,
        }],
        audio_streams: Vec::new(),
    }
}

fn request(format: FrameCaptureFormat, timestamp: FrameTimestamp) -> FrameCaptureRequest {
    FrameCaptureRequest {
        input: PathBuf::from("source/input.mkv"),
        output: PathBuf::from(format!("output/frame.{}", format.extension())),
        timestamp,
        format,
    }
}

#[test]
fn formats_own_extensions_and_image_muxer() {
    assert_eq!(FrameCaptureFormat::Png.extension(), "png");
    assert_eq!(FrameCaptureFormat::Jpeg.extension(), "jpg");
    assert_eq!(FrameCaptureFormat::Png.ffmpeg_muxer_name(), "image2");
    assert_eq!(FrameCaptureFormat::Jpeg.ffmpeg_muxer_name(), "image2");
}

#[test]
fn missing_video_and_negative_timestamps_are_rejected() {
    let mut no_video = media(None);
    no_video.video_streams.clear();
    no_video
        .streams
        .retain(|stream| stream.kind != StreamKind::Video);
    assert_eq!(
        validate_frame_capture_request(
            &request(FrameCaptureFormat::Png, FrameTimestamp::ZERO),
            &no_video
        ),
        Err(FrameCaptureValidationError::MissingVideoStream)
    );

    assert_eq!(
        validate_frame_capture_request(
            &request(
                FrameCaptureFormat::Png,
                FrameTimestamp::from_milliseconds(-1)
            ),
            &media(None)
        ),
        Err(FrameCaptureValidationError::NegativeTimestamp)
    );
}

#[test]
fn known_duration_rejects_equal_and_later_timestamps() {
    let media = media(Some(Duration::from_millis(10_500)));
    assert_eq!(
        validate_frame_capture_request(
            &request(
                FrameCaptureFormat::Png,
                FrameTimestamp::from_milliseconds(10_500)
            ),
            &media
        ),
        Err(FrameCaptureValidationError::TimestampAtOrBeyondDuration {
            timestamp_milliseconds: 10_500,
            duration_milliseconds: 10_500,
        })
    );
    assert_eq!(
        validate_frame_capture_request(
            &request(
                FrameCaptureFormat::Png,
                FrameTimestamp::from_milliseconds(10_501)
            ),
            &media
        ),
        Err(FrameCaptureValidationError::TimestampAtOrBeyondDuration {
            timestamp_milliseconds: 10_501,
            duration_milliseconds: 10_500,
        })
    );
}

#[test]
fn sub_millisecond_duration_precision_does_not_shrink_the_valid_boundary() {
    let media = media(Some(
        Duration::from_millis(10_500) + Duration::from_micros(500),
    ));

    assert_eq!(
        validate_frame_capture_request(
            &request(
                FrameCaptureFormat::Png,
                FrameTimestamp::from_milliseconds(10_500)
            ),
            &media
        ),
        Ok(())
    );
}

#[test]
fn missing_duration_does_not_invent_an_upper_bound() {
    assert_eq!(
        validate_frame_capture_request(
            &request(
                FrameCaptureFormat::Jpeg,
                FrameTimestamp::from_milliseconds(i64::MAX)
            ),
            &media(None)
        ),
        Ok(())
    );
}

#[test]
fn output_extension_must_match_selected_format() {
    for (format, wrong) in [
        (FrameCaptureFormat::Png, "frame.jpg"),
        (FrameCaptureFormat::Jpeg, "frame.jpeg"),
    ] {
        let mut request = request(format, FrameTimestamp::ZERO);
        request.output = PathBuf::from(wrong);
        assert!(matches!(
            validate_frame_capture_request(&request, &media(None)),
            Err(FrameCaptureValidationError::OutputExtensionMismatch { .. })
        ));
    }
}

#[test]
fn command_maps_sparse_absolute_video_index_and_outputs_exactly_one_frame_without_audio() {
    let request = request(
        FrameCaptureFormat::Png,
        FrameTimestamp::from_str("00:01:02.345").unwrap(),
    );
    let args = build_frame_capture_invocation(&request, 7).args;

    assert!(has_pair(&args, "-ss", "00:01:02.345"));
    assert!(has_pair(&args, "-map", "0:7"));
    assert_eq!(args.iter().filter(|arg| *arg == "-map").count(), 1);
    assert!(has_pair(&args, "-frames:v", "1"));
    assert_eq!(args.iter().filter(|arg| *arg == "-frames:v").count(), 1);
    assert!(args.iter().any(|arg| arg == "-an"));
    assert!(has_pair(&args, "-abort_on", "empty_output"));
    assert!(
        !args
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("0:a"))
    );
    assert_eq!(
        args.last().map(OsString::as_os_str),
        Some(request.output.as_os_str())
    );
}

#[test]
fn png_and_jpeg_commands_use_explicit_codec_and_quality_policies() {
    let png =
        build_frame_capture_invocation(&request(FrameCaptureFormat::Png, FrameTimestamp::ZERO), 7)
            .args;
    assert!(has_pair(&png, "-c:v", "png"));
    assert!(has_pair(&png, "-f", "image2"));
    assert!(!png.iter().any(|arg| arg == "-q:v"));

    let jpeg =
        build_frame_capture_invocation(&request(FrameCaptureFormat::Jpeg, FrameTimestamp::ZERO), 7)
            .args;
    assert!(has_pair(&jpeg, "-c:v", "mjpeg"));
    assert!(has_pair(&jpeg, "-q:v", "2"));
    assert!(has_pair(&jpeg, "-f", "image2"));
}

fn has_pair(args: &[OsString], option: &str, value: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == option && pair[1] == value)
}
