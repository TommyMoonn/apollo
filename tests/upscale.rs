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
