use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use apollo::{
    media::{
        model::{AudioStream, MediaFile, VideoStream},
        output::MediaOutputError,
    },
    upscale::{
        command::{UpscaleCommandError, build_upscale_invocation},
        model::{UpscaleRequest, UpscaleTarget, VideoCodec},
        sizing::{
            OutputDimensions, UpscaleSizingError, calculate_output_dimensions,
            oriented_target_boundary,
        },
        validation::{UpscaleValidationError, validate_upscale_request},
    },
};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn supported_targets_build_the_planned_scale_filters() {
    let temp = TestDirectory::new();
    let media = media_with_audio_codecs(
        temp.path().join("input.mp4"),
        Some(1280),
        Some(720),
        &[Some("aac")],
    );

    for (target, expected_filter) in [
        (UpscaleTarget::Qhd1440, "scale=2560:1440:flags=lanczos"),
        (UpscaleTarget::Uhd4k, "scale=3840:2160:flags=lanczos"),
        (UpscaleTarget::Uhd8k, "scale=7680:4320:flags=lanczos"),
    ] {
        let request = request(&temp, target);
        let dimensions = calculate_output_dimensions(1280, 720, target)
            .expect("fixture should have a valid upscale");
        let args = invocation_args(&request, &media, dimensions);

        assert_eq!(option_values(&args, "-vf"), vec![expected_filter]);
    }
}

#[test]
fn h264_uses_libx264_crf_18_and_slow_preset() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac")],
    );
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-c:v"), vec!["libx264"]);
    assert_eq!(option_values(&args, "-crf"), vec!["18"]);
    assert_eq!(option_values(&args, "-preset"), vec!["slow"]);
}

#[test]
fn h265_uses_libx265_crf_20_and_slow_preset() {
    let temp = TestDirectory::new();
    let mut request = request(&temp, UpscaleTarget::Uhd4k);
    request.codec = VideoCodec::H265;
    let media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac")],
    );
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-c:v"), vec!["libx265"]);
    assert_eq!(option_values(&args, "-crf"), vec!["20"]);
    assert_eq!(option_values(&args, "-preset"), vec!["slow"]);
}

#[test]
fn calculated_ultrawide_output_is_used_without_padding_or_crop() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media =
        media_with_audio_codecs(request.input.clone(), Some(1920), Some(800), &[Some("aac")]);
    let dimensions = calculate_output_dimensions(1920, 800, request.target)
        .expect("ultrawide fixture should have a valid upscale");
    let args = invocation_args(&request, &media, dimensions);
    let filters = option_values(&args, "-vf");

    assert_eq!(filters, vec!["scale=3840:1600:flags=lanczos"]);
    assert!(!filters[0].contains("pad="));
    assert!(!filters[0].contains("crop="));
}

#[test]
fn video_is_reencoded_and_mp4_output_is_explicit() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac")],
    );
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-c:v"), vec!["libx264"]);
    assert_ne!(option_values(&args, "-c:v"), vec!["copy"]);
    assert_eq!(option_values(&args, "-f"), vec!["mp4"]);
}

#[test]
fn primary_video_and_optional_audio_are_mapped_explicitly() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let mut media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac")],
    );
    media.video_streams[0].stream_index = 5;
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-map"), vec!["0:5", "0:a?"]);
}

#[test]
fn mp4_compatible_audio_codecs_are_stream_copied() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);

    for codec in [
        "aac",
        "alac",
        "ac3",
        "eac3",
        "mp3",
        "opus",
        "flac",
        "vorbis",
        "dts",
        "pcm_s16le",
        "pcm_s16be",
        "pcm_s24le",
        "pcm_s24be",
        "pcm_s32le",
        "pcm_s32be",
    ] {
        let media = media_with_audio_codecs(
            request.input.clone(),
            Some(1920),
            Some(1080),
            &[Some(codec)],
        );
        let args = invocation_args(
            &request,
            &media,
            OutputDimensions {
                width: 3840,
                height: 2160,
            },
        );

        assert_eq!(option_values(&args, "-c:a"), vec!["copy"], "codec {codec}");
    }
}

#[test]
fn incompatible_or_unknown_audio_falls_back_to_aac() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);

    for codec in [Some("wmav2"), Some("not-a-real-codec"), None] {
        let media =
            media_with_audio_codecs(request.input.clone(), Some(1920), Some(1080), &[codec]);
        let args = invocation_args(
            &request,
            &media,
            OutputDimensions {
                width: 3840,
                height: 2160,
            },
        );

        assert_eq!(option_values(&args, "-c:a"), vec!["aac"]);
    }
}

#[test]
fn any_incompatible_audio_stream_forces_safe_aac_fallback() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac"), Some("wmav2")],
    );
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-c:a"), vec!["aac"]);
}

#[test]
fn no_audio_keeps_optional_mapping_without_audio_codec_option() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media(request.input.clone(), Some(1920), Some(1080));
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-map"), vec!["0:0", "0:a?"]);
    assert!(option_values(&args, "-c:a").is_empty());
}

#[test]
fn invocation_requests_machine_readable_progress_on_stdout() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac")],
    );
    let args = invocation_args(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    );

    assert_eq!(option_values(&args, "-progress"), vec!["pipe:1"]);
    assert!(args.iter().any(|arg| arg == "-nostats"));
}

#[test]
fn unusual_input_and_output_paths_remain_single_arguments() {
    let temp = TestDirectory::new();
    let request = UpscaleRequest {
        input: temp.path().join("source [$weird]; name 'one'.mkv"),
        output: temp.path().join("output [$weird]; name 'two'.mp4"),
        target: UpscaleTarget::Uhd4k,
        codec: VideoCodec::H264,
    };
    let media = media_with_audio_codecs(
        request.input.clone(),
        Some(1920),
        Some(1080),
        &[Some("aac")],
    );
    let invocation = build_upscale_invocation(
        &request,
        &media,
        OutputDimensions {
            width: 3840,
            height: 2160,
        },
    )
    .expect("valid request should build an FFmpeg invocation");

    assert_eq!(invocation.args[0], "-i");
    assert_eq!(invocation.args[1], request.input.as_os_str());
    assert_eq!(
        invocation.args.last().map(OsString::as_os_str),
        Some(request.output.as_os_str())
    );
    assert_eq!(
        invocation
            .args
            .iter()
            .filter(|arg| arg.as_os_str() == request.input.as_os_str())
            .count(),
        1
    );
    assert_eq!(
        invocation
            .args
            .iter()
            .filter(|arg| arg.as_os_str() == request.output.as_os_str())
            .count(),
        1
    );
}

#[test]
fn command_builder_rejects_missing_primary_video_stream() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = MediaFile {
        path: request.input.clone(),
        duration: None,
        video_streams: Vec::new(),
        audio_streams: Vec::new(),
    };

    assert_eq!(
        build_upscale_invocation(
            &request,
            &media,
            OutputDimensions {
                width: 3840,
                height: 2160,
            },
        ),
        Err(UpscaleCommandError::MissingVideoStream)
    );
}

#[test]
fn supported_targets_expose_the_planned_boundaries() {
    assert_eq!(UpscaleTarget::Qhd1440.boundary(), (2560, 1440));
    assert_eq!(UpscaleTarget::Uhd4k.boundary(), (3840, 2160));
    assert_eq!(UpscaleTarget::Uhd8k.boundary(), (7680, 4320));
}

#[test]
fn sixteen_by_nine_fills_the_4k_boundary() {
    assert_eq!(
        calculate_output_dimensions(1920, 1080, UpscaleTarget::Uhd4k),
        Ok(OutputDimensions {
            width: 3840,
            height: 2160,
        })
    );
}

#[test]
fn ultrawide_fills_the_long_edge_without_padding_or_crop() {
    assert_eq!(
        calculate_output_dimensions(1920, 800, UpscaleTarget::Uhd4k),
        Ok(OutputDimensions {
            width: 3840,
            height: 1600,
        })
    );
}

#[test]
fn four_by_three_fills_the_short_edge_without_stretching() {
    assert_eq!(
        calculate_output_dimensions(1024, 768, UpscaleTarget::Uhd4k),
        Ok(OutputDimensions {
            width: 2880,
            height: 2160,
        })
    );
}

#[test]
fn portrait_uses_the_corresponding_portrait_boundary() {
    assert_eq!(
        oriented_target_boundary(UpscaleTarget::Uhd4k, 1080, 1920),
        OutputDimensions {
            width: 2160,
            height: 3840,
        }
    );
    assert_eq!(
        calculate_output_dimensions(1080, 1920, UpscaleTarget::Uhd4k),
        Ok(OutputDimensions {
            width: 2160,
            height: 3840,
        })
    );
}

#[test]
fn square_uses_the_landscape_box_and_is_limited_by_its_short_edge() {
    assert_eq!(
        oriented_target_boundary(UpscaleTarget::Uhd4k, 1000, 1000),
        OutputDimensions {
            width: 3840,
            height: 2160,
        }
    );
    assert_eq!(
        calculate_output_dimensions(1000, 1000, UpscaleTarget::Uhd4k),
        Ok(OutputDimensions {
            width: 2160,
            height: 2160,
        })
    );
}

#[test]
fn odd_source_dimensions_preserve_exact_ratio_when_even_multiple_fits() {
    let output = calculate_output_dimensions(1503, 1002, UpscaleTarget::Uhd4k)
        .expect("odd-sized source should have a valid even output");

    assert_eq!(
        output,
        OutputDimensions {
            width: 3240,
            height: 2160,
        }
    );
    assert_eq!(
        u64::from(output.width) * 1002,
        u64::from(output.height) * 1503
    );
    assert!(output.width.is_multiple_of(2));
    assert!(output.height.is_multiple_of(2));
}

#[test]
fn nonstandard_854_by_480_uses_the_maximal_4k_fit() {
    let output = calculate_output_dimensions(854, 480, UpscaleTarget::Uhd4k)
        .expect("non-standard source should have a valid 4K fit");

    assert_eq!(
        output,
        OutputDimensions {
            width: 3840,
            height: 2158,
        }
    );
    assert_close_to_maximal_boundary_fit(854, 480, UpscaleTarget::Uhd4k, output);
}

#[test]
fn nonstandard_1001_by_333_uses_the_maximal_4k_fit() {
    let output = calculate_output_dimensions(1001, 333, UpscaleTarget::Uhd4k)
        .expect("non-standard source should have a valid 4K fit");

    assert_eq!(
        output,
        OutputDimensions {
            width: 3840,
            height: 1278,
        }
    );
    assert_close_to_maximal_boundary_fit(1001, 333, UpscaleTarget::Uhd4k, output);
}

#[test]
fn portrait_nonstandard_ratio_uses_the_maximal_portrait_4k_fit() {
    let output = calculate_output_dimensions(480, 854, UpscaleTarget::Uhd4k)
        .expect("portrait source should have a valid 4K fit");

    assert_eq!(
        output,
        OutputDimensions {
            width: 2158,
            height: 3840,
        }
    );
    assert_close_to_maximal_boundary_fit(480, 854, UpscaleTarget::Uhd4k, output);
}

#[test]
fn even_rounding_never_exceeds_the_selected_boundary() {
    for (width, height) in [(1919, 1079), (2001, 2000), (1079, 1919), (1001, 333)] {
        let boundary = oriented_target_boundary(UpscaleTarget::Uhd4k, width, height);
        let output = calculate_output_dimensions(width, height, UpscaleTarget::Uhd4k)
            .expect("fixture should produce a valid upscale");

        assert!(output.width <= boundary.width);
        assert!(output.height <= boundary.height);
        assert!(output.width.is_multiple_of(2));
        assert!(output.height.is_multiple_of(2));
    }
}

#[test]
fn source_at_target_boundary_is_rejected_by_sizing() {
    assert_eq!(
        calculate_output_dimensions(3840, 2160, UpscaleTarget::Uhd4k),
        Err(UpscaleSizingError::TargetNotLarger {
            target: UpscaleTarget::Uhd4k,
            source_width: 3840,
            source_height: 2160,
            boundary_width: 3840,
            boundary_height: 2160,
        })
    );
}

#[test]
fn sizing_rejects_zero_source_dimensions() {
    assert_eq!(
        calculate_output_dimensions(0, 1080, UpscaleTarget::Uhd4k),
        Err(UpscaleSizingError::InvalidSourceDimensions {
            width: 0,
            height: 1080,
        })
    );
}

#[test]
fn even_quantization_never_downscales_a_source_edge() {
    for (width, height) in [(3839, 2159), (3000, 1), (2159, 3839), (2001, 2000)] {
        let output = calculate_output_dimensions(width, height, UpscaleTarget::Uhd4k)
            .expect("fixture should produce a valid upscale");

        assert!(output.width >= width);
        assert!(output.height >= height);
    }
}

#[test]
fn near_boundary_source_uses_minimal_even_fallback_when_exact_upscale_cannot_fit() {
    assert_eq!(
        calculate_output_dimensions(3839, 2159, UpscaleTarget::Uhd4k),
        Ok(OutputDimensions {
            width: 3840,
            height: 2160,
        })
    );
}

#[test]
fn aspect_ratio_error_is_limited_to_even_pixel_quantization() {
    for (width, height) in [
        (1920, 1080),
        (1920, 800),
        (1024, 768),
        (1080, 1920),
        (1000, 1000),
        (1919, 1079),
        (3839, 2159),
        (2001, 2000),
        (1079, 1919),
        (854, 480),
        (1001, 333),
        (480, 854),
    ] {
        let output = calculate_output_dimensions(width, height, UpscaleTarget::Uhd4k)
            .expect("fixture should produce a valid upscale");

        assert_close_to_maximal_boundary_fit(width, height, UpscaleTarget::Uhd4k, output);
    }
}

#[test]
fn each_supported_target_accepts_a_smaller_source() {
    let temp = TestDirectory::new();
    let media = media(temp.path().join("input.mp4"), Some(1280), Some(720));

    for target in [
        UpscaleTarget::Qhd1440,
        UpscaleTarget::Uhd4k,
        UpscaleTarget::Uhd8k,
    ] {
        let request = request(&temp, target);
        assert_eq!(validate_upscale_request(&request, &media), Ok(()));
    }
}

#[test]
fn missing_video_stream_is_rejected() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = MediaFile {
        path: request.input.clone(),
        duration: None,
        video_streams: Vec::new(),
        audio_streams: Vec::new(),
    };

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::MissingVideoStream)
    );
}

#[test]
fn missing_source_dimensions_are_rejected() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media(request.input.clone(), Some(1920), None);

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::MissingSourceDimensions { stream_index: 0 })
    );
}

#[test]
fn zero_source_dimensions_are_rejected() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media(request.input.clone(), Some(0), Some(1080));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::InvalidSourceDimensions {
            width: 0,
            height: 1080,
        })
    );
}

#[test]
fn same_input_and_output_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = request(&temp, UpscaleTarget::Uhd4k);
    fs::write(&request.input, b"input").expect("fixture input should be created");
    request.output = request.input.clone();
    let media = media(request.input.clone(), Some(1920), Some(1080));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::Output(
            MediaOutputError::SameInputAndOutput(
                fs::canonicalize(temp.path()).unwrap().join("input.mp4")
            )
        ))
    );
}

#[test]
fn source_at_target_boundary_is_rejected() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let media = media(request.input.clone(), Some(3840), Some(2160));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::TargetNotLarger {
            target: UpscaleTarget::Uhd4k,
            source_width: 3840,
            source_height: 2160,
            target_width: 3840,
            target_height: 2160,
        })
    );
}

#[test]
fn source_exceeding_either_target_edge_is_rejected_to_avoid_downscale() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);

    for (width, height) in [(4000, 1000), (3000, 2200)] {
        let media = media(request.input.clone(), Some(width), Some(height));
        assert!(matches!(
            validate_upscale_request(&request, &media),
            Err(UpscaleValidationError::TargetNotLarger { .. })
        ));
    }
}

#[test]
fn target_eligibility_is_orientation_independent() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    let landscape = media(request.input.clone(), Some(3000), Some(2000));
    let portrait = media(request.input.clone(), Some(2000), Some(3000));

    assert_eq!(validate_upscale_request(&request, &landscape), Ok(()));
    assert_eq!(validate_upscale_request(&request, &portrait), Ok(()));
}

#[test]
fn output_path_requires_a_usable_file_name() {
    let temp = TestDirectory::new();
    let mut request = request(&temp, UpscaleTarget::Uhd4k);
    request.output = PathBuf::new();
    let media = media(request.input.clone(), Some(1920), Some(1080));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::Output(
            MediaOutputError::InvalidOutputPath(PathBuf::new())
        ))
    );
}

#[test]
fn nonexistent_output_parent_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = request(&temp, UpscaleTarget::Uhd4k);
    let parent = temp.path().join("missing");
    request.output = parent.join("upscaled.mp4");
    let media = media(request.input.clone(), Some(1920), Some(1080));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::Output(
            MediaOutputError::OutputParentUnavailable {
                path: parent,
                kind: std::io::ErrorKind::NotFound,
            }
        ))
    );
}

#[test]
fn output_parent_must_be_a_directory() {
    let temp = TestDirectory::new();
    let parent_file = temp.path().join("not-a-directory");
    fs::write(&parent_file, b"occupied").expect("fixture file should be created");
    let mut request = request(&temp, UpscaleTarget::Uhd4k);
    request.output = parent_file.join("upscaled.mp4");
    let media = media(request.input.clone(), Some(1920), Some(1080));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::Output(
            MediaOutputError::OutputParentNotDirectory(parent_file)
        ))
    );
}

#[test]
fn existing_output_is_rejected_before_execution() {
    let temp = TestDirectory::new();
    let request = request(&temp, UpscaleTarget::Uhd4k);
    fs::write(&request.output, b"occupied").expect("fixture output should be created");
    let media = media(request.input.clone(), Some(1920), Some(1080));

    assert!(matches!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::Output(
            MediaOutputError::OutputAlreadyExists(_)
        ))
    ));
}

#[test]
fn valid_request_preserves_selected_codec_in_domain_model() {
    let temp = TestDirectory::new();
    let mut request = request(&temp, UpscaleTarget::Uhd8k);
    request.codec = VideoCodec::H265;
    let media = media(request.input.clone(), Some(3840), Some(2160));

    assert_eq!(validate_upscale_request(&request, &media), Ok(()));
    assert_eq!(request.codec, VideoCodec::H265);
}

fn assert_close_to_maximal_boundary_fit(
    source_width: u32,
    source_height: u32,
    target: UpscaleTarget,
    output: OutputDimensions,
) {
    let boundary = oriented_target_boundary(target, source_width, source_height);
    let width_limited = u64::from(boundary.width) * u64::from(source_height)
        <= u64::from(boundary.height) * u64::from(source_width);

    assert!(output.width >= source_width);
    assert!(output.height >= source_height);
    assert!(output.width <= boundary.width);
    assert!(output.height <= boundary.height);
    assert!(output.width.is_multiple_of(2));
    assert!(output.height.is_multiple_of(2));

    if width_limited {
        let ideal_height =
            f64::from(source_height) * f64::from(boundary.width) / f64::from(source_width);
        assert_eq!(output.width, boundary.width);
        assert!(
            (f64::from(output.height) - ideal_height).abs() <= 1.0,
            "height {} should be within one pixel of maximal ideal {ideal_height}",
            output.height
        );
    } else {
        let ideal_width =
            f64::from(source_width) * f64::from(boundary.height) / f64::from(source_height);
        assert_eq!(output.height, boundary.height);
        assert!(
            (f64::from(output.width) - ideal_width).abs() <= 1.0,
            "width {} should be within one pixel of maximal ideal {ideal_width}",
            output.width
        );
    }
}

fn invocation_args(
    request: &UpscaleRequest,
    media: &MediaFile,
    dimensions: OutputDimensions,
) -> Vec<String> {
    build_upscale_invocation(request, media, dimensions)
        .expect("valid request should build an FFmpeg invocation")
        .args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn option_values<'a>(args: &'a [String], option: &str) -> Vec<&'a str> {
    args.windows(2)
        .filter_map(|pair| (pair[0] == option).then_some(pair[1].as_str()))
        .collect()
}

fn request(temp: &TestDirectory, target: UpscaleTarget) -> UpscaleRequest {
    UpscaleRequest {
        input: temp.path().join("input.mp4"),
        output: temp.path().join("upscaled.mp4"),
        target,
        codec: VideoCodec::H264,
    }
}

fn media_with_audio_codecs(
    path: PathBuf,
    width: Option<u32>,
    height: Option<u32>,
    codecs: &[Option<&str>],
) -> MediaFile {
    let mut media = media(path, width, height);
    media.audio_streams = codecs
        .iter()
        .enumerate()
        .map(|(offset, codec)| AudioStream {
            stream_index: u32::try_from(offset + 1).expect("test stream index should fit u32"),
            codec: codec.map(str::to_owned),
            channels: Some(2),
            channel_layout: Some("stereo".to_owned()),
            title: None,
            language: None,
        })
        .collect();
    media
}

fn media(path: PathBuf, width: Option<u32>, height: Option<u32>) -> MediaFile {
    MediaFile {
        path,
        duration: None,
        video_streams: vec![VideoStream {
            stream_index: 0,
            codec: Some("h264".to_owned()),
            width,
            height,
            frame_rate: None,
        }],
        audio_streams: Vec::new(),
    }
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "apollo-upscale-validation-{}-{sequence}",
            process::id()
        ));

        if path.exists() {
            fs::remove_dir_all(&path).expect("stale fixture directory should be removable");
        }
        fs::create_dir(&path).expect("fixture directory should be created");

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
