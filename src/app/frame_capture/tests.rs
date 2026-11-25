use super::*;
use crate::frame_capture::preview::PreviewState;
use crate::media::probe::parse_ffprobe_json;

#[test]
fn identical_requests_from_distinct_editors_cannot_cross_settle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ffprobe/complete_inventory.json");
    let source = SourceIdentity::read(&path).unwrap();
    let media = parse_ffprobe_json(
        &path,
        include_bytes!("../../../tests/fixtures/ffprobe/complete_inventory.json"),
    )
    .unwrap();
    let mut first = FrameCaptureEditorState::new(1, Some(source.clone()), &media);
    let mut second = FrameCaptureEditorState::new(2, Some(source), &media);
    let position = first.selection.valid_position(&media).unwrap();
    let now = Instant::now();
    first
        .schedule_preview(&media, Some(position), PositionSettle::Immediate, now)
        .unwrap();
    second
        .schedule_preview(&media, Some(position), PositionSettle::Immediate, now)
        .unwrap();
    let old = first.preview.take_due_request(now).unwrap();
    let current = second.preview.take_due_request(now).unwrap();
    assert_eq!(old, current);
    let image = PreviewImage::from_rgba([1, 1], vec![255; 4]).unwrap();
    assert!(!second.settle_preview(first.session_id, old, Ok(image.clone())));
    assert!(matches!(
        second.preview.state(),
        PreviewState::Loading { .. }
    ));
    assert!(second.settle_preview(second.session_id, current, Ok(image)));

    let old = first.schedule_index(&media, position).unwrap().unwrap();
    let current = second.schedule_index(&media, position).unwrap().unwrap();
    assert_eq!(old, current);
    assert!(!second.settle_index(first.session_id, old, Err(FrameIndexError::Cancelled)));
    assert!(matches!(
        second.frame_navigator.state(),
        FrameIndexState::Loading { .. }
    ));
    second.settle_index(second.session_id, current, Err(FrameIndexError::Cancelled));
    assert!(matches!(
        second.frame_navigator.state(),
        FrameIndexState::Failed { .. }
    ));
    assert_eq!(second.selection.valid_position(&media), Some(position));
}
