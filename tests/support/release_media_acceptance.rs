//! Operation acceptance for the pair extracted from an actual release artifact.
//! Called only by the isolated no-PATH subprocess in WindowsRelease.Smoke.ps1.
use std::{
    env, fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use apollo::{
    audio_extract::{
        execution::AudioExtractExecutor,
        model::{AudioExtractFormat, AudioExtractRequest},
    },
    audio_mix::{
        execution::AudioMixExecutor,
        model::{
            AudioMixRequest, LoudnessNormalization, LoudnessTargets, OutputContainer,
            SelectedAudioTrack, TrackGain,
        },
    },
    frame_capture::{
        execution::FrameCaptureExecutor,
        model::{FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp},
    },
    media::{
        container::MediaContainer,
        execution::MediaExecutionState,
        model::MediaFile,
        probe::probe_media,
        process::{MediaProcess, MediaProcessCommand, MediaProcessState},
        tools::{MediaTools, VideoEncoderCapabilities, VideoEncoderCapability},
    },
    remux::{execution::RemuxExecutor, model::RemuxRequest},
    upscale::{
        execution::UpscaleExecutor,
        model::{
            EncoderQuality, NvencQuality, UpscaleEncoderSettings, UpscaleRequest, UpscaleTarget,
            VideoCodec, VideoEncoderBackend, VideoEncoderSelection,
        },
    },
};

pub fn run(tools: &MediaTools, capabilities: &VideoEncoderCapabilities, mode: &str) {
    assert_eq!(env::var_os("PATH").unwrap_or_default(), "");
    let output =
        env::var_os("APOLLO_RELEASE_OUTPUT_DIRECTORY").expect("acceptance output directory");
    let directory = Path::new(&output);
    fs::create_dir(directory).unwrap();
    let input = directory.join("source.mp4");
    generate_fixture(tools, &input);
    let media = probe_media(tools, &input).unwrap();
    assert_video(&media, "h264", 320, 180);
    assert_eq!(media.audio_streams.len(), 2);
    assert!(
        media
            .audio_streams
            .iter()
            .all(|stream| stream.codec.as_deref() == Some("aac"))
    );
    println!("PASS probe: one H.264 video and two AAC tracks");

    match mode {
        "cpu" => {
            audio_mix(tools, &media, directory);
            remux(tools, &media, directory);
            audio_extract(tools, &media, directory);
            frame_capture(tools, &media, directory);
            upscale(
                tools,
                &media,
                directory,
                capabilities,
                VideoEncoderSelection::Cpu,
            );
        }
        "nvenc" => {
            assert_eq!(capabilities.h264_nvenc, VideoEncoderCapability::Available);
            assert_eq!(capabilities.h265_nvenc, VideoEncoderCapability::Available);
            upscale(
                tools,
                &media,
                directory,
                capabilities,
                VideoEncoderSelection::NvidiaNvenc,
            );
            upscale(
                tools,
                &media,
                directory,
                capabilities,
                VideoEncoderSelection::Auto,
            );
        }
        "unavailable" => {
            assert_eq!(env::var("CUDA_VISIBLE_DEVICES").unwrap(), "-1");
            for capability in [&capabilities.h264_nvenc, &capabilities.h265_nvenc] {
                let VideoEncoderCapability::RuntimeUnavailable { reason } = capability else {
                    panic!("expected real NVENC probe failure, got {capability:?}");
                };
                assert!(!reason.trim().is_empty());
                println!("Retained real NVENC failure: {reason}");
            }
            upscale(
                tools,
                &media,
                directory,
                capabilities,
                VideoEncoderSelection::Auto,
            );
        }
        _ => panic!("unknown acceptance mode: {mode}"),
    }
    println!("PASS release acceptance: {mode}");
}

fn generate_fixture(tools: &MediaTools, input: &Path) {
    // Only source synthesis is test-owned. Every user operation below uses its production executor.
    let process = MediaProcess::start(
        MediaProcessCommand::new(&tools.ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=12:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:duration=1",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "2:a",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-t",
                "1",
            ])
            .arg(input),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !process.state().is_terminal() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    if !process.state().is_terminal() {
        process.cancel();
        panic!("fixture generation timed out");
    }
    assert_eq!(
        process.state(),
        MediaProcessState::Completed,
        "{}",
        process.diagnostic_stderr()
    );
}

fn settle(mut poll: impl FnMut() -> MediaExecutionState, output: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = poll();
        if state.is_terminal() {
            assert_eq!(
                state,
                MediaExecutionState::Completed {
                    output: output.to_path_buf()
                }
            );
            assert!(fs::metadata(output).unwrap().len() > 0);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "operation timed out: {}",
            output.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_video(media: &MediaFile, codec: &str, width: u32, height: u32) {
    assert_eq!(media.video_streams.len(), 1);
    let video = &media.video_streams[0];
    assert_eq!(video.codec.as_deref(), Some(codec));
    assert_eq!((video.width, video.height), (Some(width), Some(height)));
}

fn audio_mix(tools: &MediaTools, media: &MediaFile, directory: &Path) {
    for (label, normalization) in [
        ("plain", LoudnessNormalization::Off),
        (
            "normalized",
            LoudnessNormalization::Enabled(LoudnessTargets::default()),
        ),
    ] {
        let request = AudioMixRequest {
            input: media.path.clone(),
            output: directory.join(format!("mix-{label}.mp4")),
            selected_tracks: media
                .audio_streams
                .iter()
                .map(|track| {
                    SelectedAudioTrack::new(track.stream_index, TrackGain::from_tenths_db(-30))
                })
                .collect(),
            normalization,
            container: OutputContainer::Mp4,
        };
        let mut executor = AudioMixExecutor::new();
        executor.start(tools, media, &request).unwrap();
        settle(|| executor.poll().clone(), &request.output);
        let result = probe_media(tools, &request.output).unwrap();
        assert_video(&result, "h264", 320, 180);
        assert_eq!(result.audio_streams.len(), 1);
        assert_eq!(result.audio_streams[0].codec.as_deref(), Some("aac"));
        println!("PASS Audio Mix {label}");
    }
}

fn remux(tools: &MediaTools, media: &MediaFile, directory: &Path) {
    let request = RemuxRequest {
        input: media.path.clone(),
        output: directory.join("remux.mkv"),
        container: MediaContainer::Matroska,
    };
    let mut executor = RemuxExecutor::new();
    executor.start(tools, media, &request).unwrap();
    settle(|| executor.poll().clone(), &request.output);
    let result = probe_media(tools, &request.output).unwrap();
    assert_video(&result, "h264", 320, 180);
    assert_eq!(result.audio_streams.len(), 2);
    assert_eq!(
        result
            .streams
            .iter()
            .map(|s| (&s.kind, &s.codec))
            .collect::<Vec<_>>(),
        media
            .streams
            .iter()
            .map(|s| (&s.kind, &s.codec))
            .collect::<Vec<_>>()
    );
    println!("PASS Remux");
}

fn audio_extract(tools: &MediaTools, media: &MediaFile, directory: &Path) {
    for (format, codec) in [
        (AudioExtractFormat::Original, "aac"),
        (AudioExtractFormat::Aac, "aac"),
        (AudioExtractFormat::Flac, "flac"),
    ] {
        let request = AudioExtractRequest {
            input: media.path.clone(),
            output: directory.join(format!("audio.{}", format.extension())),
            audio_stream_index: media.audio_streams[1].stream_index,
            format,
        };
        let mut executor = AudioExtractExecutor::new();
        executor.start(tools, media, &request).unwrap();
        settle(|| executor.poll().clone(), &request.output);
        let result = probe_media(tools, &request.output).unwrap();
        assert!(result.video_streams.is_empty());
        assert_eq!(result.audio_streams.len(), 1);
        assert_eq!(result.audio_streams[0].codec.as_deref(), Some(codec));
        println!("PASS Audio Extract {format:?}");
    }
}

fn frame_capture(tools: &MediaTools, media: &MediaFile, directory: &Path) {
    for (format, codec) in [
        (FrameCaptureFormat::Png, "png"),
        (FrameCaptureFormat::Jpeg, "mjpeg"),
    ] {
        let request = FrameCaptureRequest {
            input: media.path.clone(),
            output: directory.join(format!("frame.{}", format.extension())),
            position: FrameTimestamp::from_milliseconds(500).into(),
            format,
        };
        let mut executor = FrameCaptureExecutor::new();
        executor.start(tools, media, &request).unwrap();
        settle(|| executor.poll().clone(), &request.output);
        let result = probe_media(tools, &request.output).unwrap();
        assert_video(&result, codec, 320, 180);
        assert!(result.audio_streams.is_empty());
        println!("PASS Frame Capture {format:?}");
    }
}

fn upscale(
    tools: &MediaTools,
    media: &MediaFile,
    directory: &Path,
    capabilities: &VideoEncoderCapabilities,
    selection: VideoEncoderSelection,
) {
    for (codec, name, capability) in [
        (VideoCodec::H264, "h264", &capabilities.h264_nvenc),
        (VideoCodec::H265, "hevc", &capabilities.h265_nvenc),
    ] {
        let mut settings = UpscaleEncoderSettings::default();
        settings.set_selection(selection);
        let request = UpscaleRequest {
            input: media.path.clone(),
            output: directory.join(format!("upscale-{selection:?}-{name}.mp4")),
            target: UpscaleTarget::Custom {
                max_width: 640,
                max_height: 360,
            },
            codec,
            encoder_settings: settings,
            nvenc_capability: capability.clone(),
        };
        let expected = if selection == VideoEncoderSelection::Cpu
            || *capability != VideoEncoderCapability::Available
        {
            VideoEncoderBackend::Cpu
        } else {
            VideoEncoderBackend::NvidiaNvenc
        };
        let resolved = request.resolve_video_encoder().unwrap();
        assert_eq!(resolved.backend, expected);
        if expected == VideoEncoderBackend::NvidiaNvenc {
            assert_eq!(
                resolved.quality,
                EncoderQuality::NvidiaNvenc(NvencQuality::new(16).unwrap())
            );
        }
        if let VideoEncoderCapability::RuntimeUnavailable { reason } = capability {
            let mut explicit = request.clone();
            explicit
                .encoder_settings
                .set_selection(VideoEncoderSelection::NvidiaNvenc);
            let error = UpscaleExecutor::new()
                .start(tools, media, &explicit)
                .unwrap_err();
            assert!(error.to_string().contains(reason));
            assert!(!explicit.output.exists());
            println!(
                "PASS explicit {name} NVENC rejects real probe failure with retained diagnostic"
            );
        }
        let mut executor = UpscaleExecutor::new();
        executor.start(tools, media, &request).unwrap();
        settle(|| executor.poll().clone(), &request.output);
        let result = probe_media(tools, &request.output).unwrap();
        assert_video(&result, name, 640, 360);
        assert_eq!(result.video_streams[0].format.bit_depth, Some(8));
        assert_eq!(result.audio_streams.len(), 2);
        println!("PASS Upscale {name}: selection={selection:?}, resolved={resolved:?}");
    }
}
