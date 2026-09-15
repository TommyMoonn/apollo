use std::{
    cmp::Ordering, collections::VecDeque, ffi::OsString, fmt, path::PathBuf, time::Duration,
};

use serde::Deserialize;

use crate::media::{
    execution::ProcessControl,
    intake::SourceIdentity,
    model::MediaFile,
    process::{
        MediaProcess, MediaProcessCommand, MediaProcessFailure, MediaProcessStartError,
        MediaProcessState, MediaProcessWake,
    },
    tools::MediaTools,
};

use super::{
    model::{FramePosition, FrameTimeBase, ResolvedFramePosition},
    validation::validate_frame_position,
};

const QUERY_RADIUS_NANOSECONDS: i64 = 1_500_000_000;
const QUERY_DURATION_NANOSECONDS: i64 = QUERY_RADIUS_NANOSECONDS * 2;
const QUERY_STDOUT_LIMIT_BYTES: usize = 2 * 1024 * 1024;
const MAX_NEIGHBORHOOD_FRAMES: usize = 4_096;
const NEIGHBORHOOD_CACHE_CAPACITY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameIndexGeneration(u64);

impl FrameIndexGeneration {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameQueryInterval {
    start_nanoseconds: i64,
    duration_nanoseconds: i64,
}

impl FrameQueryInterval {
    fn around(position: FramePosition, duration: Option<Duration>) -> Option<Self> {
        let center = position.nanoseconds()?.max(0);
        let start_nanoseconds = center.saturating_sub(QUERY_RADIUS_NANOSECONDS).max(0);
        let mut duration_nanoseconds = QUERY_DURATION_NANOSECONDS;
        if let Some(duration) = duration {
            let duration = i64::try_from(duration.as_nanos()).ok()?;
            duration_nanoseconds =
                duration_nanoseconds.min(duration.saturating_sub(start_nanoseconds));
        }
        (duration_nanoseconds > 0).then_some(Self {
            start_nanoseconds,
            duration_nanoseconds,
        })
    }

    pub const fn start_nanoseconds(self) -> i64 {
        self.start_nanoseconds
    }

    pub const fn duration_nanoseconds(self) -> i64 {
        self.duration_nanoseconds
    }

    fn end_nanoseconds(self) -> i64 {
        self.start_nanoseconds
            .saturating_add(self.duration_nanoseconds)
    }

    fn ffprobe_value(self) -> String {
        format!(
            "{}%{}",
            format_nanoseconds(self.start_nanoseconds),
            format_nanoseconds(self.end_nanoseconds())
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameIndexRequest {
    pub generation: FrameIndexGeneration,
    pub source_identity: SourceIdentity,
    pub input: PathBuf,
    pub video_stream_index: u32,
    pub center: FramePosition,
    pub interval: FrameQueryInterval,
    pub known_duration: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameIndexInvocation {
    pub args: Vec<OsString>,
}

pub fn build_frame_index_invocation(request: &FrameIndexRequest) -> FrameIndexInvocation {
    FrameIndexInvocation {
        args: vec![
            OsString::from("-v"),
            OsString::from("error"),
            OsString::from("-select_streams"),
            OsString::from(request.video_stream_index.to_string()),
            OsString::from("-read_intervals"),
            OsString::from(request.interval.ffprobe_value()),
            OsString::from("-show_streams"),
            OsString::from("-show_frames"),
            OsString::from("-show_entries"),
            OsString::from("stream=time_base:frame=best_effort_timestamp,pts"),
            OsString::from("-of"),
            OsString::from("json"),
            request.input.as_os_str().to_os_string(),
        ],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameIndexError {
    MissingVideoStream,
    InvalidPosition,
    Start(MediaProcessStartError),
    Process {
        reason: MediaProcessFailure,
        diagnostic: String,
    },
    Cancelled,
    InvalidOutput(String),
    MissingTimeBase,
    TooManyFrames,
    NoFrames,
}

impl fmt::Display for FrameIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => formatter.write_str("source has no video stream"),
            Self::InvalidPosition => formatter.write_str("frame position is outside the source"),
            Self::Start(error) => write!(formatter, "frame timing query could not start: {error}"),
            Self::Process { reason, .. } => {
                write!(formatter, "frame timing query failed: {reason:?}")
            }
            Self::Cancelled => formatter.write_str("frame timing query was cancelled"),
            Self::InvalidOutput(error) => write!(formatter, "invalid frame timing data: {error}"),
            Self::MissingTimeBase => formatter.write_str("frame timing data has no time base"),
            Self::TooManyFrames => formatter.write_str("frame timing neighborhood is too large"),
            Self::NoFrames => formatter.write_str("no video frames were found near this position"),
        }
    }
}

impl std::error::Error for FrameIndexError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameNeighborhood {
    frames: Vec<ResolvedFramePosition>,
    selected: usize,
    start_reached: bool,
    end_reached: bool,
}

impl FrameNeighborhood {
    pub fn frames(&self) -> &[ResolvedFramePosition] {
        &self.frames
    }

    pub fn selected(&self) -> ResolvedFramePosition {
        self.frames[self.selected]
    }

    pub const fn start_reached(&self) -> bool {
        self.start_reached
    }

    pub const fn end_reached(&self) -> bool {
        self.end_reached
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStep {
    One,
    Two,
    Five,
    Ten,
}

impl FrameStep {
    const fn count(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Five => 5,
            Self::Ten => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameNavigation {
    Resolved(ResolvedFramePosition),
    NeedsNeighborhood(FramePosition),
    Boundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameIndexState {
    Idle,
    Loading {
        request: FrameIndexRequest,
    },
    Ready {
        request: FrameIndexRequest,
        neighborhood: FrameNeighborhood,
    },
    Failed {
        request: FrameIndexRequest,
        error: FrameIndexError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameNeighborhoodCacheKey {
    source_identity: SourceIdentity,
    video_stream_index: u32,
    center: FramePosition,
    interval: FrameQueryInterval,
}

impl From<&FrameIndexRequest> for FrameNeighborhoodCacheKey {
    fn from(request: &FrameIndexRequest) -> Self {
        Self {
            source_identity: request.source_identity.clone(),
            video_stream_index: request.video_stream_index,
            center: request.center,
            interval: request.interval,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameNavigator {
    generation: FrameIndexGeneration,
    state: FrameIndexState,
    source_identity: Option<SourceIdentity>,
    video_stream_index: Option<u32>,
    cache: VecDeque<(FrameNeighborhoodCacheKey, FrameNeighborhood)>,
}

impl Default for FrameNavigator {
    fn default() -> Self {
        Self {
            generation: FrameIndexGeneration(0),
            state: FrameIndexState::Idle,
            source_identity: None,
            video_stream_index: None,
            cache: VecDeque::new(),
        }
    }
}

impl FrameNavigator {
    pub fn state(&self) -> &FrameIndexState {
        &self.state
    }

    pub fn schedule(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        center: FramePosition,
    ) -> Result<Option<FrameIndexRequest>, FrameIndexError> {
        let video_stream_index = media
            .video_streams
            .first()
            .ok_or(FrameIndexError::MissingVideoStream)?
            .stream_index;
        if validate_frame_position(center, media.duration).is_err() {
            return Err(FrameIndexError::InvalidPosition);
        }
        let interval = FrameQueryInterval::around(center, media.duration)
            .ok_or(FrameIndexError::InvalidPosition)?;

        if self.source_identity.as_ref() != Some(source_identity)
            || self.video_stream_index != Some(video_stream_index)
        {
            self.cache.clear();
            self.source_identity = Some(source_identity.clone());
            self.video_stream_index = Some(video_stream_index);
        }

        let request = FrameIndexRequest {
            generation: self.advance_generation(),
            source_identity: source_identity.clone(),
            input: media.path.clone(),
            video_stream_index,
            center,
            interval,
            known_duration: media.duration,
        };
        if let Some(neighborhood) = self.cached(&request) {
            self.state = FrameIndexState::Ready {
                request,
                neighborhood,
            };
            return Ok(None);
        }
        self.state = FrameIndexState::Loading {
            request: request.clone(),
        };
        Ok(Some(request))
    }

    pub fn settle(
        &mut self,
        request: FrameIndexRequest,
        result: Result<Vec<u8>, FrameIndexError>,
    ) -> bool {
        if !matches!(&self.state, FrameIndexState::Loading { request: current } if current == &request)
        {
            return false;
        }
        self.state = match result.and_then(|bytes| parse_neighborhood(&request, &bytes)) {
            Ok(neighborhood) => {
                self.cache_insert(&request, neighborhood.clone());
                FrameIndexState::Ready {
                    request,
                    neighborhood,
                }
            }
            Err(error) => FrameIndexState::Failed { request, error },
        };
        true
    }

    pub fn navigate(
        &self,
        current: FramePosition,
        direction: FrameDirection,
        step: FrameStep,
    ) -> FrameNavigation {
        let Some(coverage) = self.contiguous_coverage(current) else {
            return FrameNavigation::NeedsNeighborhood(current);
        };
        let mut frames = self
            .cache
            .iter()
            .enumerate()
            .filter(|(index, _)| coverage[*index])
            .flat_map(|(_, (_, neighborhood))| neighborhood.frames.iter().copied())
            .collect::<Vec<_>>();
        frames.sort_by(|left, right| left.compare(*right));
        frames.dedup_by(|left, right| left.compare(*right) == Ordering::Equal);
        if frames.is_empty() {
            return FrameNavigation::NeedsNeighborhood(current);
        }

        let selected = frames
            .iter()
            .position(|frame| frame.compare_position(current) != Ordering::Less)
            .unwrap_or(frames.len() - 1);
        let count = step.count();
        let target = match direction {
            FrameDirection::Previous => selected.checked_sub(count),
            FrameDirection::Next => selected
                .checked_add(count)
                .filter(|index| *index < frames.len()),
        };
        if let Some(target) = target {
            return FrameNavigation::Resolved(frames[target]);
        }

        let boundary = match direction {
            FrameDirection::Previous => self
                .cache
                .iter()
                .enumerate()
                .any(|(index, (_, neighborhood))| coverage[index] && neighborhood.start_reached),
            FrameDirection::Next => self
                .cache
                .iter()
                .enumerate()
                .any(|(index, (_, neighborhood))| coverage[index] && neighborhood.end_reached),
        };
        if boundary {
            FrameNavigation::Boundary
        } else {
            let anchor = match direction {
                FrameDirection::Previous => frames[0],
                FrameDirection::Next => *frames.last().expect("nonempty frames checked above"),
            };
            FrameNavigation::NeedsNeighborhood(anchor.into())
        }
    }

    pub fn invalidate(&mut self) {
        self.advance_generation();
        self.state = FrameIndexState::Idle;
    }

    fn contiguous_coverage(&self, current: FramePosition) -> Option<Vec<bool>> {
        let position = current.nanoseconds()?;
        let owner = self.cache.iter().position(|(key, _)| {
            key.interval.start_nanoseconds() <= position
                && position <= key.interval.end_nanoseconds()
        })?;
        let mut included = vec![false; self.cache.len()];
        included[owner] = true;
        let mut start = self.cache[owner].0.interval.start_nanoseconds();
        let mut end = self.cache[owner].0.interval.end_nanoseconds();

        loop {
            let mut extended = false;
            for (index, (key, _)) in self.cache.iter().enumerate() {
                if !included[index]
                    && key.interval.start_nanoseconds() <= end
                    && key.interval.end_nanoseconds() >= start
                {
                    included[index] = true;
                    start = start.min(key.interval.start_nanoseconds());
                    end = end.max(key.interval.end_nanoseconds());
                    extended = true;
                }
            }
            if !extended {
                return Some(included);
            }
        }
    }

    fn advance_generation(&mut self) -> FrameIndexGeneration {
        self.generation.0 = self
            .generation
            .0
            .checked_add(1)
            .expect("Frame Capture frame-index generation space is exhausted");
        self.generation
    }

    fn cached(&mut self, request: &FrameIndexRequest) -> Option<FrameNeighborhood> {
        let key = FrameNeighborhoodCacheKey::from(request);
        let index = self
            .cache
            .iter()
            .position(|(candidate, _)| *candidate == key)?;
        let entry = self
            .cache
            .remove(index)
            .expect("located frame-neighborhood cache entry remains present");
        let neighborhood = entry.1.clone();
        self.cache.push_back(entry);
        Some(neighborhood)
    }

    fn cache_insert(&mut self, request: &FrameIndexRequest, neighborhood: FrameNeighborhood) {
        let key = FrameNeighborhoodCacheKey::from(request);
        if let Some(index) = self
            .cache
            .iter()
            .position(|(candidate, _)| *candidate == key)
        {
            self.cache.remove(index);
        }
        self.cache.push_back((key, neighborhood));
        while self.cache.len() > NEIGHBORHOOD_CACHE_CAPACITY {
            self.cache.pop_front();
        }
    }

    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

#[derive(Debug, Deserialize)]
struct RawFrameIndex {
    #[serde(default)]
    frames: Vec<RawFrame>,
    #[serde(default)]
    streams: Vec<RawStream>,
}

#[derive(Debug, Deserialize)]
struct RawFrame {
    best_effort_timestamp: Option<i64>,
    pts: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RawStream {
    time_base: Option<String>,
}

fn parse_neighborhood(
    request: &FrameIndexRequest,
    bytes: &[u8],
) -> Result<FrameNeighborhood, FrameIndexError> {
    let raw: RawFrameIndex = serde_json::from_slice(bytes)
        .map_err(|error| FrameIndexError::InvalidOutput(error.to_string()))?;
    if raw.frames.len() > MAX_NEIGHBORHOOD_FRAMES {
        return Err(FrameIndexError::TooManyFrames);
    }
    let time_base = raw
        .streams
        .first()
        .and_then(|stream| stream.time_base.as_deref())
        .and_then(parse_time_base)
        .ok_or(FrameIndexError::MissingTimeBase)?;
    let mut frames = raw
        .frames
        .into_iter()
        .filter_map(|frame| frame.best_effort_timestamp.or(frame.pts))
        .filter_map(|timestamp| ResolvedFramePosition::new(timestamp, time_base))
        .filter(|frame| {
            FramePosition::from(*frame)
                .nanoseconds()
                .is_some_and(|value| {
                    value >= request.interval.start_nanoseconds()
                        && value <= request.interval.end_nanoseconds()
                })
        })
        .collect::<Vec<_>>();
    frames.sort_by(|left, right| left.compare(*right));
    frames.dedup_by(|left, right| left.compare(*right) == Ordering::Equal);
    if frames.is_empty() {
        return Err(FrameIndexError::NoFrames);
    }
    let selected = frames
        .iter()
        .position(|frame| frame.compare_position(request.center) != Ordering::Less)
        .unwrap_or(frames.len() - 1);
    let duration_reached = request.known_duration.is_some_and(|duration| {
        i64::try_from(duration.as_nanos())
            .is_ok_and(|duration| request.interval.end_nanoseconds() >= duration)
    });
    let end_reached = duration_reached
        || (request.known_duration.is_none()
            && frames
                .last()
                .is_some_and(|frame| frame.compare_position(request.center) != Ordering::Greater));
    Ok(FrameNeighborhood {
        frames,
        selected,
        start_reached: request.interval.start_nanoseconds() == 0,
        end_reached,
    })
}

fn parse_time_base(value: &str) -> Option<FrameTimeBase> {
    let (numerator, denominator) = value.split_once('/')?;
    FrameTimeBase::new(numerator.parse().ok()?, denominator.parse().ok()?)
}

fn format_nanoseconds(nanoseconds: i64) -> String {
    let seconds = nanoseconds / 1_000_000_000;
    let fraction = nanoseconds % 1_000_000_000;
    format!("{seconds}.{fraction:09}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameIndexCompletion {
    pub request: FrameIndexRequest,
    pub result: Result<Vec<u8>, FrameIndexError>,
}

pub struct FrameIndexQuery {
    request: FrameIndexRequest,
    process: Option<Box<dyn FrameIndexProcessControl>>,
}

#[derive(Debug, Default)]
pub struct FrameIndexQueryOwner {
    active: Option<FrameIndexQuery>,
}

impl FrameIndexQueryOwner {
    pub fn start(
        &mut self,
        tools: &MediaTools,
        request: FrameIndexRequest,
        wake: MediaProcessWake,
    ) -> Result<(), FrameIndexError> {
        self.cancel();
        self.active = Some(
            FrameIndexQuery::start_with_wake(tools, request, wake)
                .map_err(FrameIndexError::Start)?,
        );
        Ok(())
    }

    pub fn poll(&mut self) -> Option<FrameIndexCompletion> {
        let completion = self.active.as_mut()?.poll()?;
        self.active = None;
        Some(completion)
    }

    pub fn cancel(&mut self) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        active.cancel()
    }

    #[cfg(test)]
    fn replace_for_test(&mut self, query: FrameIndexQuery) {
        self.cancel();
        self.active = Some(query);
    }
}

trait FrameIndexProcessControl: ProcessControl {
    fn captured_stdout_bytes(&self) -> Vec<u8>;
}

impl FrameIndexProcessControl for MediaProcess {
    fn captured_stdout_bytes(&self) -> Vec<u8> {
        MediaProcess::captured_stdout_bytes(self)
    }
}

impl fmt::Debug for FrameIndexQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameIndexQuery")
            .field("request", &self.request)
            .field("running", &self.process.is_some())
            .finish()
    }
}

impl FrameIndexQuery {
    pub fn start_with_wake(
        tools: &MediaTools,
        request: FrameIndexRequest,
        wake: MediaProcessWake,
    ) -> Result<Self, MediaProcessStartError> {
        Self::start_with_launcher(tools, request, move |command| {
            MediaProcess::start_with_wake(command, wake)
        })
    }

    pub fn cancel(&self) -> bool {
        self.process
            .as_ref()
            .is_some_and(|process| process.cancel())
    }

    pub fn poll(&mut self) -> Option<FrameIndexCompletion> {
        let state = self.process.as_ref()?.state();
        if state == MediaProcessState::Running {
            return None;
        }
        let process = self
            .process
            .take()
            .expect("frame timing process exists while terminal output is settled");
        let result = match state {
            MediaProcessState::Completed => Ok(process.captured_stdout_bytes()),
            MediaProcessState::Failed(reason) => Err(FrameIndexError::Process {
                reason,
                diagnostic: process.diagnostic_stderr(),
            }),
            MediaProcessState::Cancelled => Err(FrameIndexError::Cancelled),
            MediaProcessState::Running => unreachable!("running query was returned above"),
        };
        Some(FrameIndexCompletion {
            request: self.request.clone(),
            result,
        })
    }

    fn start_with_launcher<F, P>(
        tools: &MediaTools,
        request: FrameIndexRequest,
        launcher: F,
    ) -> Result<Self, MediaProcessStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: FrameIndexProcessControl + 'static,
    {
        let invocation = build_frame_index_invocation(&request);
        let process = launcher(
            MediaProcessCommand::new(&tools.ffprobe)
                .args(invocation.args)
                .captured_stdout_limit(QUERY_STDOUT_LIMIT_BYTES),
        )?;
        Ok(Self {
            request,
            process: Some(Box::new(process)),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use crate::media::{
        execution::ProcessControl,
        intake::SourceIdentity,
        model::{MediaFile, VideoStream},
        process::{MediaProcessFailure, MediaProcessState},
        tools::MediaTools,
    };

    use super::*;

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn query_maps_only_primary_stream_and_uses_a_bounded_interval() {
        let (directory, media, identity) = source("query");
        let mut navigator = FrameNavigator::default();
        let request = navigator
            .schedule(&identity, &media, requested(12_500))
            .unwrap()
            .unwrap();
        let invocation = build_frame_index_invocation(&request);

        assert!(has_pair(&invocation.args, "-select_streams", "7"));
        assert_eq!(
            invocation
                .args
                .iter()
                .filter(|arg| *arg == "-select_streams")
                .count(),
            1
        );
        assert!(has_pair(
            &invocation.args,
            "-read_intervals",
            "11.000000000%14.000000000"
        ));
        assert!(invocation.args.iter().any(|arg| arg == "-show_streams"));
        assert!(invocation.args.iter().any(|arg| arg == "-show_frames"));
        assert_eq!(invocation.args.last().unwrap(), media.path.as_os_str());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn vfr_navigation_uses_returned_pts_for_all_supported_step_sizes() {
        let (directory, media, identity) = source("vfr");
        let mut navigator = FrameNavigator::default();
        let request = navigator
            .schedule(&identity, &media, requested(100))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(
            request,
            Ok(frame_json(
                "1/1000",
                &[
                    0, 40, 100, 170, 240, 400, 620, 900, 1200, 1400, 1600, 1800, 2000
                ]
            ))
        ));

        for (step, expected) in [
            (FrameStep::One, 170),
            (FrameStep::Two, 240),
            (FrameStep::Five, 900),
            (FrameStep::Ten, 2000),
        ] {
            assert_eq!(
                navigator.navigate(requested(100), FrameDirection::Next, step),
                FrameNavigation::Resolved(resolved(expected, 1, 1000))
            );
        }
        assert_eq!(
            navigator.navigate(requested(900), FrameDirection::Previous, FrameStep::Five),
            FrameNavigation::Resolved(resolved(100, 1, 1000))
        );
        for (step, expected) in [
            (FrameStep::One, 1600),
            (FrameStep::Two, 1400),
            (FrameStep::Five, 620),
            (FrameStep::Ten, 40),
        ] {
            assert_eq!(
                navigator.navigate(
                    resolved(1800, 1, 1000).into(),
                    FrameDirection::Previous,
                    step
                ),
                FrameNavigation::Resolved(resolved(expected, 1, 1000))
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn high_frame_rate_navigation_uses_exact_neighbor_timestamps() {
        let (directory, media, identity) = source("hfr");
        let mut navigator = FrameNavigator::default();
        let request = navigator
            .schedule(&identity, &media, resolved(10_010, 1, 120_000).into())
            .unwrap()
            .unwrap();
        let timestamps = (0..=20).map(|index| index * 1_001).collect::<Vec<_>>();
        assert!(navigator.settle(request, Ok(frame_json("1/120000", &timestamps))));

        assert_eq!(
            navigator.navigate(
                resolved(10_010, 1, 120_000).into(),
                FrameDirection::Next,
                FrameStep::Five,
            ),
            FrameNavigation::Resolved(resolved(15_015, 1, 120_000))
        );
        assert_eq!(
            navigator.navigate(
                resolved(10_010, 1, 120_000).into(),
                FrameDirection::Previous,
                FrameStep::Ten,
            ),
            FrameNavigation::Resolved(resolved(0, 1, 120_000))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn fractional_rate_frames_retain_exact_pts_and_boundaries_do_not_wrap() {
        let (directory, mut media, identity) = source("fractional");
        media.duration = Some(Duration::from_millis(101));
        let mut navigator = FrameNavigator::default();
        let request = navigator
            .schedule(&identity, &media, requested(0))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(request, Ok(frame_json("1/30000", &[0, 1001, 2002, 3003]))));

        assert_eq!(
            navigator.navigate(requested(0), FrameDirection::Previous, FrameStep::One),
            FrameNavigation::Boundary
        );
        assert_eq!(
            navigator.navigate(
                resolved(3003, 1, 30000).into(),
                FrameDirection::Next,
                FrameStep::One
            ),
            FrameNavigation::Boundary
        );
        assert_eq!(resolved(1001, 1, 30000).ffmpeg_timestamp(), "0.033366666");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn leaving_a_local_neighborhood_requests_an_adjacent_bounded_query() {
        let (directory, media, identity) = source("adjacent");
        let mut navigator = FrameNavigator::default();
        let request = navigator
            .schedule(&identity, &media, requested(5_000))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(
            request,
            Ok(frame_json(
                "1/1000",
                &[3500, 4000, 4500, 5000, 5500, 6000, 6500]
            ))
        ));

        assert_eq!(
            navigator.navigate(requested(6_500), FrameDirection::Next, FrameStep::Ten),
            FrameNavigation::NeedsNeighborhood(resolved(6500, 1, 1000).into())
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn disjoint_neighborhoods_do_not_become_adjacent_frame_sequences() {
        let (directory, media, identity) = source("disjoint");
        let mut navigator = FrameNavigator::default();
        let earlier = navigator
            .schedule(&identity, &media, requested(3_000))
            .unwrap()
            .unwrap();
        let earlier_frames = (1_500..=4_500).step_by(250).collect::<Vec<_>>();
        assert!(navigator.settle(earlier, Ok(frame_json("1/1000", &earlier_frames))));
        let later = navigator
            .schedule(&identity, &media, requested(20_000))
            .unwrap()
            .unwrap();
        let later_frames = (18_500..=21_500).step_by(250).collect::<Vec<_>>();
        assert!(navigator.settle(later, Ok(frame_json("1/1000", &later_frames))));

        for step in [FrameStep::One, FrameStep::Ten] {
            assert_eq!(
                navigator.navigate(requested(4_500), FrameDirection::Next, step),
                FrameNavigation::NeedsNeighborhood(resolved(4500, 1, 1000).into())
            );
            assert_eq!(
                navigator.navigate(requested(18_500), FrameDirection::Previous, step),
                FrameNavigation::NeedsNeighborhood(resolved(18500, 1, 1000).into())
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn boundary_flags_apply_only_to_the_current_contiguous_coverage() {
        let (directory, media, identity) = source("disjoint-boundaries");
        let mut navigator = FrameNavigator::default();
        let start = navigator
            .schedule(&identity, &media, requested(500))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(start, Ok(frame_json("1/1000", &[0, 500, 1000]))));
        let middle = navigator
            .schedule(&identity, &media, requested(20_000))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(
            middle,
            Ok(frame_json("1/1000", &[19000, 19500, 20000, 20500, 21000]))
        ));
        let end = navigator
            .schedule(&identity, &media, requested(59_000))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(end, Ok(frame_json("1/1000", &[59000, 59500]))));

        assert_eq!(
            navigator.navigate(requested(19_000), FrameDirection::Previous, FrameStep::Ten),
            FrameNavigation::NeedsNeighborhood(resolved(19000, 1, 1000).into())
        );
        assert_eq!(
            navigator.navigate(requested(21_000), FrameDirection::Next, FrameStep::Ten),
            FrameNavigation::NeedsNeighborhood(resolved(21000, 1, 1000).into())
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn overlapping_neighborhoods_extend_known_navigation_coverage() {
        let (directory, media, identity) = source("overlap");
        let mut navigator = FrameNavigator::default();
        let earlier = navigator
            .schedule(&identity, &media, requested(5_000))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(
            earlier,
            Ok(frame_json("1/1000", &[3500, 4000, 4500, 5000, 5500, 6000]))
        ));
        let overlapping = navigator
            .schedule(&identity, &media, requested(6_000))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(
            overlapping,
            Ok(frame_json(
                "1/1000",
                &[4500, 5000, 5500, 6000, 6500, 7000, 7500]
            ))
        ));

        assert_eq!(
            navigator.navigate(requested(6_000), FrameDirection::Next, FrameStep::One),
            FrameNavigation::Resolved(resolved(6500, 1, 1000))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_duration_uses_discovered_frame_availability_as_the_end_boundary() {
        let (directory, mut media, identity) = source("unknown-duration");
        media.duration = None;
        let mut navigator = FrameNavigator::default();
        let request = navigator
            .schedule(&identity, &media, requested(500))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(request, Ok(frame_json("1/1000", &[0, 250, 500]))));

        assert_eq!(
            navigator.navigate(requested(500), FrameDirection::Next, FrameStep::One),
            FrameNavigation::Boundary
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cache_hit_avoids_query_and_source_replacement_clears_old_neighborhoods() {
        let (directory, media, identity) = source("cache");
        let mut navigator = FrameNavigator::default();
        let first = navigator
            .schedule(&identity, &media, requested(2_000))
            .unwrap()
            .unwrap();
        assert!(navigator.settle(first, Ok(frame_json("1/1000", &[1000, 2000, 3000]))));
        assert!(
            navigator
                .schedule(&identity, &media, requested(2_000))
                .unwrap()
                .is_none()
        );

        fs::write(&media.path, b"replacement source with new identity").unwrap();
        let replacement = SourceIdentity::read(&media.path).unwrap();
        assert!(
            navigator
                .schedule(&replacement, &media, requested(2_000))
                .unwrap()
                .is_some()
        );
        assert_eq!(navigator.cache_len(), 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn neighborhood_cache_has_a_deterministic_explicit_bound() {
        let (directory, media, identity) = source("bounded-cache");
        let mut navigator = FrameNavigator::default();
        for index in 0..=NEIGHBORHOOD_CACHE_CAPACITY {
            let center = 5_000 + index as i64 * 4_000;
            let request = navigator
                .schedule(&identity, &media, requested(center))
                .unwrap()
                .unwrap();
            assert!(navigator.settle(request, Ok(frame_json("1/1000", &[center]))));
        }
        assert_eq!(navigator.cache_len(), NEIGHBORHOOD_CACHE_CAPACITY);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_completion_is_rejected_and_process_failure_retains_diagnostics() {
        let (directory, media, identity) = source("stale");
        let mut navigator = FrameNavigator::default();
        let stale = navigator
            .schedule(&identity, &media, requested(1_000))
            .unwrap()
            .unwrap();
        let current = navigator
            .schedule(&identity, &media, requested(2_000))
            .unwrap()
            .unwrap();
        assert!(!navigator.settle(stale, Ok(frame_json("1/1000", &[1000]))));
        assert!(
            matches!(navigator.state(), FrameIndexState::Loading { request } if request == &current)
        );

        let process = FakeProcess::failed(MediaProcessFailure::Exit(Some(3)), "ffprobe details");
        let mut query = FrameIndexQuery::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            current.clone(),
            |_| Ok(process),
        )
        .unwrap();
        assert_eq!(
            query.poll().unwrap().result,
            Err(FrameIndexError::Process {
                reason: MediaProcessFailure::Exit(Some(3)),
                diagnostic: "ffprobe details".to_owned(),
            })
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn completed_query_returns_exact_bytes_with_its_original_request() {
        let (directory, media, identity) = source("completion");
        let request = FrameNavigator::default()
            .schedule(&identity, &media, requested(1_000))
            .unwrap()
            .unwrap();
        let payload = frame_json("1/1000", &[900, 1000, 1100]);
        let process = FakeProcess::completed(payload.clone());
        let mut query = FrameIndexQuery::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            request.clone(),
            |_| Ok(process),
        )
        .unwrap();

        assert_eq!(
            query.poll(),
            Some(FrameIndexCompletion {
                request,
                result: Ok(payload),
            })
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn query_owner_replacement_cancels_the_prior_process() {
        let (directory, media, identity) = source("owner-replacement");
        let mut navigator = FrameNavigator::default();
        let first_request = navigator
            .schedule(&identity, &media, requested(1_000))
            .unwrap()
            .unwrap();
        let second_request = navigator
            .schedule(&identity, &media, requested(2_000))
            .unwrap()
            .unwrap();
        let first_process = FakeProcess::running();
        let first_handle = first_process.clone();
        let first_query = FrameIndexQuery::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            first_request,
            |_| Ok(first_process),
        )
        .unwrap();
        let second_process = FakeProcess::running();
        let second_handle = second_process.clone();
        let second_query = FrameIndexQuery::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            second_request,
            |_| Ok(second_process),
        )
        .unwrap();
        let mut owner = FrameIndexQueryOwner::default();

        owner.replace_for_test(first_query);
        owner.replace_for_test(second_query);

        assert_eq!(first_handle.state(), MediaProcessState::Cancelled);
        assert_eq!(second_handle.state(), MediaProcessState::Running);
        assert!(owner.cancel());
        assert_eq!(second_handle.state(), MediaProcessState::Cancelled);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn active_query_is_cancellable_through_the_media_process_owner() {
        let (directory, media, identity) = source("cancel");
        let request = FrameNavigator::default()
            .schedule(&identity, &media, requested(1_000))
            .unwrap()
            .unwrap();
        let process = FakeProcess::running();
        let handle = process.clone();
        let mut query = FrameIndexQuery::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            request,
            |_| Ok(process),
        )
        .unwrap();

        assert!(query.cancel());
        assert_eq!(handle.state(), MediaProcessState::Cancelled);
        assert_eq!(
            query.poll().unwrap().result,
            Err(FrameIndexError::Cancelled)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn source(name: &str) -> (PathBuf, MediaFile, SourceIdentity) {
        let id = NEXT_DIRECTORY.fetch_add(1, AtomicOrdering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("apollo-frame-index-{name}-{nonce}-{id}"));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("source.mkv");
        fs::write(&path, b"source").unwrap();
        let identity = SourceIdentity::read(&path).unwrap();
        let media = MediaFile {
            path,
            duration: Some(Duration::from_secs(60)),
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 7,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: Vec::new(),
        };
        (directory, media, identity)
    }

    fn requested(milliseconds: i64) -> FramePosition {
        crate::frame_capture::model::FrameTimestamp::from_milliseconds(milliseconds).into()
    }

    fn resolved(timestamp: i64, numerator: u32, denominator: u32) -> ResolvedFramePosition {
        ResolvedFramePosition::new(
            timestamp,
            FrameTimeBase::new(numerator, denominator).unwrap(),
        )
        .unwrap()
    }

    fn frame_json(time_base: &str, timestamps: &[i64]) -> Vec<u8> {
        let frames = timestamps
            .iter()
            .map(|timestamp| format!(r#"{{"best_effort_timestamp":{timestamp}}}"#))
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"frames":[{frames}],"streams":[{{"time_base":"{time_base}"}}]}}"#).into_bytes()
    }

    fn has_pair(args: &[OsString], option: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == option && pair[1] == value)
    }

    #[derive(Clone)]
    struct FakeProcess {
        state: Arc<Mutex<MediaProcessState>>,
        diagnostic: String,
        stdout: Vec<u8>,
    }

    impl FakeProcess {
        fn running() -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
                diagnostic: String::new(),
                stdout: Vec::new(),
            }
        }

        fn completed(stdout: Vec<u8>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Completed)),
                diagnostic: String::new(),
                stdout,
            }
        }

        fn failed(reason: MediaProcessFailure, diagnostic: &str) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Failed(reason))),
                diagnostic: diagnostic.to_owned(),
                stdout: Vec::new(),
            }
        }
    }

    impl ProcessControl for FakeProcess {
        fn state(&self) -> MediaProcessState {
            self.state.lock().unwrap().clone()
        }

        fn cancel(&self) -> bool {
            let mut state = self.state.lock().unwrap();
            if *state != MediaProcessState::Running {
                return false;
            }
            *state = MediaProcessState::Cancelled;
            true
        }

        fn diagnostic_stderr(&self) -> String {
            self.diagnostic.clone()
        }

        fn drain_progress(&self) -> Vec<String> {
            Vec::new()
        }
    }

    impl FrameIndexProcessControl for FakeProcess {
        fn captured_stdout_bytes(&self) -> Vec<u8> {
            self.stdout.clone()
        }
    }
}
