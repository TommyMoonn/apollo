use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use apollo::{
    media::model::{MediaFile, VideoStream},
    upscale::{
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
    request.output = request.input.clone();
    let media = media(request.input.clone(), Some(1920), Some(1080));

    assert_eq!(
        validate_upscale_request(&request, &media),
        Err(UpscaleValidationError::SameInputAndOutput(
            request.output.clone()
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
        Err(UpscaleValidationError::InvalidOutputPath(PathBuf::new()))
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
        Err(UpscaleValidationError::OutputParentUnavailable {
            path: parent,
            kind: std::io::ErrorKind::NotFound,
        })
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
        Err(UpscaleValidationError::OutputParentNotDirectory(
            parent_file
        ))
    );
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

fn request(temp: &TestDirectory, target: UpscaleTarget) -> UpscaleRequest {
    UpscaleRequest {
        input: temp.path().join("input.mp4"),
        output: temp.path().join("upscaled.mp4"),
        target,
        codec: VideoCodec::H264,
    }
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
