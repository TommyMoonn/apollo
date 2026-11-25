use super::*;
use crate::media::{model::MediaFile, probe::parse_ffprobe_json};

fn media() -> MediaFile {
    parse_ffprobe_json(
        Path::new("media/recording.mkv"),
        include_bytes!("../../../tests/fixtures/ffprobe/complete_inventory.json"),
    )
    .unwrap()
}

fn assert_request(
    draft: OutputDraft<'_>,
    target: DialogTarget,
    format: OutputFormat,
    file_name: &str,
) {
    let FileDialogRequest::SaveOutput {
        target: actual,
        options,
    } = draft.request(7)
    else {
        panic!("expected save request");
    };
    assert_eq!(actual, target);
    assert_eq!(options.initial_directory, Some(PathBuf::from("media")));
    assert_eq!(options.file_name, file_name);
    assert_eq!(options.format, format);
}

#[test]
fn audio_mix_and_remux_save_requests_follow_container() {
    let media = media();
    let mut mix = AudioMixDraft::new(&media);
    let mix_target = DialogTarget::AudioMixOutput { session_id: 7 };
    assert_request(
        OutputDraft::AudioMix(&mix),
        mix_target,
        OutputFormat::Mp4,
        "recording-mixed.mp4",
    );
    mix.set_container(&media, OutputContainer::Mkv);
    assert_request(
        OutputDraft::AudioMix(&mix),
        mix_target,
        OutputFormat::Mkv,
        "recording-mixed.mkv",
    );
    let mut remux = RemuxDraft::new(&media);
    let remux_target = DialogTarget::RemuxOutput { session_id: 7 };
    assert_request(
        OutputDraft::Remux(&remux),
        remux_target,
        OutputFormat::Mkv,
        "recording-remuxed.mkv",
    );
    remux.set_container(MediaContainer::Mp4);
    assert_request(
        OutputDraft::Remux(&remux),
        remux_target,
        OutputFormat::Mp4,
        "recording-remuxed.mp4",
    );
}

#[test]
fn extraction_and_capture_save_requests_follow_format() {
    let media = media();
    let mut audio = AudioExtractDraft::new(&media);
    for (format, filter, file) in [
        (
            AudioExtractFormat::Original,
            OutputFormat::Mka,
            "recording-audio.mka",
        ),
        (
            AudioExtractFormat::Aac,
            OutputFormat::M4a,
            "recording-audio.m4a",
        ),
        (
            AudioExtractFormat::Flac,
            OutputFormat::Flac,
            "recording-audio.flac",
        ),
    ] {
        audio.set_format(format);
        assert_request(
            OutputDraft::AudioExtract(&audio),
            DialogTarget::AudioExtractOutput { session_id: 7 },
            filter,
            file,
        );
    }
    let mut frame = FrameCaptureDraft::new(&media);
    for (format, filter, file) in [
        (
            FrameCaptureFormat::Png,
            OutputFormat::Png,
            "recording-frame-00-00-00-000.png",
        ),
        (
            FrameCaptureFormat::Jpeg,
            OutputFormat::Jpeg,
            "recording-frame-00-00-00-000.jpg",
        ),
    ] {
        frame.set_format(format);
        assert_request(
            OutputDraft::FrameCapture(&frame),
            DialogTarget::FrameCaptureOutput { session_id: 7 },
            filter,
            file,
        );
    }
}

#[test]
fn upscale_save_request_preserves_custom_directory_and_authoritative_extension() {
    let mut draft = UpscaleDraft::new(&media());
    draft.set_custom_output_path(PathBuf::from("chosen/output.mkv"));
    let FileDialogRequest::SaveOutput { target, options } = OutputDraft::Upscale(&draft).request(7)
    else {
        panic!("expected save request");
    };
    assert_eq!(target, DialogTarget::UpscaleOutput { session_id: 7 });
    assert_eq!(options.initial_directory, Some(PathBuf::from("chosen")));
    assert_eq!(options.file_name, "output.mp4");
    assert_eq!(options.format, OutputFormat::Mp4);
}
