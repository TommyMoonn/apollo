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

#[test]
fn native_input_results_preserve_the_accepted_source_and_directory_on_cancel_failure_or_stale() {
    use crate::{
        app::{
            ShellOrigin,
            discovery::{MediaToolsDiscovery, MediaToolsState},
        },
        media::{error::MediaErrorKind, intake::SourceIdentity, tools::MediaTools},
    };
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source file.mkv");
    std::fs::write(&source, b"fixture").unwrap();
    let mut app = Application {
        discovery: MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
            "missing-ffmpeg",
            "missing-ffprobe",
        ))),
        ..Application::default()
    };
    let mut retained = media();
    retained.path = source.clone();
    app.source
        .restore(SourceIdentity::read(&source).unwrap(), retained);
    app.enter_workspace(Operation::Remux);
    let session = app.source.session_id();
    let origin = InputDialogOrigin::capture(&app);
    let complete = |app: &mut Application, outcome| {
        app.settle_native_dialog(
            origin,
            FileDialogCompletion {
                target: DialogTarget::BatchInput,
                outcome,
            },
            || {},
        )
    };
    complete(&mut app, DialogOutcome::Cancelled);
    assert_eq!(app.source.session_id(), session);
    assert!(app.settings.source_directory().is_none());
    complete(&mut app, DialogOutcome::Failed("picker error".into()));
    assert_eq!(
        app.source.error().unwrap().kind(),
        MediaErrorKind::FilePickerUnavailable
    );
    assert_eq!(app.source.session_id(), session);
    assert!(app.settings.source_directory().is_none());
    // Re-selecting the unchanged source must reuse its inspection, rather than launching another probe.
    complete(&mut app, DialogOutcome::SelectedMany(vec![source.clone()]));
    assert_eq!(app.source.session_id(), session);
    assert_eq!(app.source.current().unwrap().path, source);
    assert!(!app.source.is_inspecting());
    assert!(app.source.error().is_none());
    assert_eq!(
        app.settings.source_directory().as_deref(),
        Some(root.path())
    );
    app.show_queue(ShellOrigin::SourceWorkspace {
        active_operation: Operation::Remux,
    });
    app.return_from_queue();
    let replacement = root.path().join("other");
    std::fs::create_dir(&replacement).unwrap();
    let replacement = replacement.join("stale.mkv");
    complete(&mut app, DialogOutcome::SelectedMany(vec![replacement]));
    assert_eq!(app.source.session_id(), session);
    assert_eq!(
        app.settings.source_directory().as_deref(),
        Some(root.path())
    );
    assert!(app.source.error().is_none());
}

#[test]
fn native_multi_selection_retains_every_path_in_batch_order() {
    use crate::app::discovery::{MediaToolsDiscovery, MediaToolsState};
    use crate::media::tools::MediaTools;
    let root = tempfile::tempdir().unwrap();
    let paths = vec![
        root.path().join("nguồn một.mkv"),
        root.path().join("source two.mp4"),
    ];
    for path in &paths {
        std::fs::write(path, b"fixture").unwrap();
    }
    let mut app = Application {
        discovery: MediaToolsDiscovery::with_state(MediaToolsState::Ready(MediaTools::new(
            "missing-ffmpeg",
            "missing-ffprobe",
        ))),
        ..Application::default()
    };
    let origin = InputDialogOrigin::capture(&app);
    app.settle_native_dialog(
        origin,
        FileDialogCompletion {
            target: DialogTarget::BatchInput,
            outcome: DialogOutcome::SelectedMany(paths.clone()),
        },
        || {},
    );
    assert_eq!(app.navigation(), crate::app::NavigationState::BatchSetup);
    let batch = app.batch_setup.as_ref().unwrap();
    assert_eq!(
        batch
            .items
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>(),
        paths
    );
    assert!(app.source.current().is_none());
    assert_eq!(
        app.settings.source_directory().as_deref(),
        Some(root.path())
    );
    app.retire_batch();
}

#[test]
fn native_output_completion_targets_the_original_editor_and_clears_picker_failure_on_success() {
    use crate::app::{
        batch::{BatchEditState, BatchEditTarget, BatchItemId},
        editor::OperationEditorState,
    };
    let root = tempfile::tempdir().unwrap();
    let mut app = Application::default();
    let editor = OperationEditorState::new(1, None, media(), Operation::Remux);
    let before = editor.remux.output_path();
    app.batch_edit = Some(BatchEditState {
        target: BatchEditTarget::Item(BatchItemId(0)),
        editor,
    });
    app.show_batch_edit();
    let origin = InputDialogOrigin::capture(&app);
    let complete = |app: &mut Application, session_id, outcome| {
        app.settle_native_dialog(
            origin,
            FileDialogCompletion {
                target: DialogTarget::RemuxOutput { session_id },
                outcome,
            },
            || {},
        )
    };
    complete(&mut app, 1, DialogOutcome::Cancelled);
    assert_eq!(app.batch_edit.as_ref().unwrap().remux.output_path(), before);
    complete(
        &mut app,
        1,
        DialogOutcome::Failed("save picker unavailable".into()),
    );
    assert!(app.batch_edit.as_ref().unwrap().error.is_some());
    assert_eq!(app.batch_edit.as_ref().unwrap().remux.output_path(), before);
    assert!(
        app.settings
            .output_directory(Path::new("missing"))
            .is_none()
    );
    let selected = root.path().join("sortie vidéo.mkv");
    complete(&mut app, 99, DialogOutcome::Selected(selected.clone()));
    assert_eq!(app.batch_edit.as_ref().unwrap().remux.output_path(), before);
    assert!(
        app.settings
            .output_directory(Path::new("missing"))
            .is_none()
    );
    complete(&mut app, 1, DialogOutcome::Selected(selected.clone()));
    assert_eq!(
        app.batch_edit.as_ref().unwrap().remux.output_path(),
        selected
    );
    assert!(app.batch_edit.as_ref().unwrap().error.is_none());
    assert_eq!(
        app.settings
            .output_directory(Path::new("missing"))
            .as_deref(),
        Some(root.path())
    );
}
