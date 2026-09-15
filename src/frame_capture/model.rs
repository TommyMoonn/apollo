use std::{cmp::Ordering, fmt, path::PathBuf, str::FromStr, time::Duration};

const MILLIS_PER_SECOND: i64 = 1_000;
const MILLIS_PER_MINUTE: i64 = 60 * MILLIS_PER_SECOND;
const MILLIS_PER_HOUR: i64 = 60 * MILLIS_PER_MINUTE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameCaptureFormat {
    Png,
    Jpeg,
}

impl FrameCaptureFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    pub const fn ffmpeg_muxer_name(self) -> &'static str {
        "image2"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FrameTimestamp {
    milliseconds: i64,
}

impl FrameTimestamp {
    pub const ZERO: Self = Self::from_milliseconds(0);

    pub const fn from_milliseconds(milliseconds: i64) -> Self {
        Self { milliseconds }
    }

    pub const fn milliseconds(self) -> i64 {
        self.milliseconds
    }

    pub fn filename_component(self) -> String {
        self.to_string().replace([':', '.'], "-")
    }
}

impl fmt::Display for FrameTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let negative = self.milliseconds < 0;
        let total = self.milliseconds.unsigned_abs();
        let hours = total / MILLIS_PER_HOUR as u64;
        let minutes = total % MILLIS_PER_HOUR as u64 / MILLIS_PER_MINUTE as u64;
        let seconds = total % MILLIS_PER_MINUTE as u64 / MILLIS_PER_SECOND as u64;
        let milliseconds = total % MILLIS_PER_SECOND as u64;

        if negative {
            formatter.write_str("-")?;
        }
        write!(
            formatter,
            "{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}"
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameTimestampParseError {
    InvalidFormat,
    ComponentOutOfRange,
    Overflow,
}

impl fmt::Display for FrameTimestampParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("timestamp must use HH:MM:SS.mmm"),
            Self::ComponentOutOfRange => {
                formatter.write_str("timestamp minutes and seconds must be between 00 and 59")
            }
            Self::Overflow => formatter.write_str("timestamp is too large"),
        }
    }
}

impl std::error::Error for FrameTimestampParseError {}

impl FromStr for FrameTimestamp {
    type Err = FrameTimestampParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (negative, value) = value
            .strip_prefix('-')
            .map_or((false, value), |value| (true, value));
        let mut time_parts = value.split(':');
        let hours = time_parts
            .next()
            .ok_or(FrameTimestampParseError::InvalidFormat)?;
        let minutes = time_parts
            .next()
            .ok_or(FrameTimestampParseError::InvalidFormat)?;
        let second_fraction = time_parts
            .next()
            .ok_or(FrameTimestampParseError::InvalidFormat)?;
        if time_parts.next().is_some() || hours.len() < 2 || minutes.len() != 2 {
            return Err(FrameTimestampParseError::InvalidFormat);
        }

        let (seconds, milliseconds) = second_fraction
            .split_once('.')
            .ok_or(FrameTimestampParseError::InvalidFormat)?;
        if seconds.len() != 2
            || milliseconds.len() != 3
            || ![hours, minutes, seconds, milliseconds]
                .into_iter()
                .all(|component| component.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(FrameTimestampParseError::InvalidFormat);
        }

        let hours = hours
            .parse::<i64>()
            .map_err(|_| FrameTimestampParseError::Overflow)?;
        let minutes = minutes
            .parse::<i64>()
            .map_err(|_| FrameTimestampParseError::Overflow)?;
        let seconds = seconds
            .parse::<i64>()
            .map_err(|_| FrameTimestampParseError::Overflow)?;
        let milliseconds = milliseconds
            .parse::<i64>()
            .map_err(|_| FrameTimestampParseError::Overflow)?;
        if minutes >= 60 || seconds >= 60 {
            return Err(FrameTimestampParseError::ComponentOutOfRange);
        }

        let total = hours
            .checked_mul(MILLIS_PER_HOUR)
            .and_then(|total| total.checked_add(minutes * MILLIS_PER_MINUTE))
            .and_then(|total| total.checked_add(seconds * MILLIS_PER_SECOND))
            .and_then(|total| total.checked_add(milliseconds))
            .ok_or(FrameTimestampParseError::Overflow)?;
        let total = if negative {
            total
                .checked_neg()
                .ok_or(FrameTimestampParseError::Overflow)?
        } else {
            total
        };
        Ok(Self::from_milliseconds(total))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameTimeBase {
    numerator: u32,
    denominator: u32,
}

impl FrameTimeBase {
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        Some(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolvedFramePosition {
    stream_timestamp: i64,
    time_base: FrameTimeBase,
}

impl ResolvedFramePosition {
    pub fn new(stream_timestamp: i64, time_base: FrameTimeBase) -> Option<Self> {
        let position = Self {
            stream_timestamp,
            time_base,
        };
        position.milliseconds().is_some().then_some(position)
    }

    pub const fn stream_timestamp(self) -> i64 {
        self.stream_timestamp
    }

    pub const fn time_base(self) -> FrameTimeBase {
        self.time_base
    }

    pub fn display_timestamp(self) -> FrameTimestamp {
        FrameTimestamp::from_milliseconds(
            self.milliseconds()
                .expect("resolved frame positions are constructed only when displayable"),
        )
    }

    pub fn ffmpeg_timestamp(self) -> String {
        format_rational_seconds(
            i128::from(self.stream_timestamp) * i128::from(self.time_base.numerator),
            i128::from(self.time_base.denominator),
        )
    }

    fn milliseconds(self) -> Option<i64> {
        let units = i128::from(self.stream_timestamp) * i128::from(self.time_base.numerator);
        let milliseconds = units.checked_mul(i128::from(MILLIS_PER_SECOND))?
            / i128::from(self.time_base.denominator);
        i64::try_from(milliseconds).ok()
    }

    fn compare_duration(self, duration: Duration) -> Ordering {
        let position = i128::from(self.stream_timestamp)
            * i128::from(self.time_base.numerator)
            * 1_000_000_000_i128;
        let duration = i128::try_from(duration.as_nanos()).expect("Duration nanoseconds fit i128")
            * i128::from(self.time_base.denominator);
        position.cmp(&duration)
    }
}

impl ResolvedFramePosition {
    pub(crate) fn compare(self, other: Self) -> Ordering {
        let left = i128::from(self.stream_timestamp)
            * i128::from(self.time_base.numerator)
            * i128::from(other.time_base.denominator);
        let right = i128::from(other.stream_timestamp)
            * i128::from(other.time_base.numerator)
            * i128::from(self.time_base.denominator);
        left.cmp(&right)
    }

    pub(crate) fn compare_position(self, other: FramePosition) -> Ordering {
        let (other_numerator, other_denominator) = other.rational_parts();
        let left = i128::from(self.stream_timestamp)
            * i128::from(self.time_base.numerator)
            * other_denominator;
        let right = other_numerator * i128::from(self.time_base.denominator);
        left.cmp(&right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FramePosition {
    Requested(FrameTimestamp),
    Resolved(ResolvedFramePosition),
}

impl FramePosition {
    pub fn display_timestamp(self) -> FrameTimestamp {
        match self {
            Self::Requested(timestamp) => timestamp,
            Self::Resolved(position) => position.display_timestamp(),
        }
    }

    pub fn ffmpeg_timestamp(self) -> String {
        match self {
            Self::Requested(timestamp) => timestamp.to_string(),
            Self::Resolved(position) => position.ffmpeg_timestamp(),
        }
    }

    pub fn is_negative(self) -> bool {
        match self {
            Self::Requested(timestamp) => timestamp.milliseconds() < 0,
            Self::Resolved(position) => position.stream_timestamp < 0,
        }
    }

    pub fn is_at_or_beyond(self, duration: Duration) -> bool {
        match self {
            Self::Requested(timestamp) => {
                timestamp.milliseconds() >= 0
                    && Duration::from_millis(timestamp.milliseconds() as u64) >= duration
            }
            Self::Resolved(position) => position.compare_duration(duration) != Ordering::Less,
        }
    }

    pub(crate) fn nanoseconds(self) -> Option<i64> {
        let (numerator, denominator) = self.rational_parts();
        i64::try_from(numerator.checked_mul(1_000_000_000)? / denominator).ok()
    }

    fn rational_parts(self) -> (i128, i128) {
        match self {
            Self::Requested(timestamp) => (
                i128::from(timestamp.milliseconds()),
                i128::from(MILLIS_PER_SECOND),
            ),
            Self::Resolved(position) => (
                i128::from(position.stream_timestamp) * i128::from(position.time_base.numerator),
                i128::from(position.time_base.denominator),
            ),
        }
    }
}

impl From<FrameTimestamp> for FramePosition {
    fn from(timestamp: FrameTimestamp) -> Self {
        Self::Requested(timestamp)
    }
}

impl From<ResolvedFramePosition> for FramePosition {
    fn from(position: ResolvedFramePosition) -> Self {
        Self::Resolved(position)
    }
}

impl fmt::Display for FramePosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display_timestamp().fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub position: FramePosition,
    pub format: FrameCaptureFormat,
}

fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn format_rational_seconds(numerator: i128, denominator: i128) -> String {
    const DECIMAL_SCALE: i128 = 1_000_000_000;
    let negative = numerator < 0;
    let numerator = numerator.abs();
    let seconds = numerator / denominator;
    let fraction = (numerator % denominator) * DECIMAL_SCALE / denominator;
    let sign = if negative { "-" } else { "" };
    format!("{sign}{seconds}.{fraction:09}")
}

#[cfg(test)]
mod tests {
    use std::{cmp::Ordering, str::FromStr};

    use super::{
        FramePosition, FrameTimeBase, FrameTimestamp, FrameTimestampParseError,
        ResolvedFramePosition,
    };

    #[test]
    fn timestamp_parse_and_format_preserve_zero_fractional_and_long_values() {
        for (text, milliseconds) in [
            ("00:00:00.000", 0),
            ("00:00:00.125", 125),
            ("01:02:03.004", 3_723_004),
            ("125:59:59.999", 453_599_999),
            ("-00:00:01.500", -1_500),
        ] {
            let timestamp = FrameTimestamp::from_str(text).unwrap();
            assert_eq!(timestamp.milliseconds(), milliseconds);
            assert_eq!(timestamp.to_string(), text);
        }
    }

    #[test]
    fn malformed_or_out_of_range_timestamps_are_rejected() {
        for text in [
            "",
            "0:00:00.000",
            "00:0:00.000",
            "00:00:0.000",
            "00:00:00",
            "00:00:00.00",
            "00:00:00.0000",
            "00:00:00.-01",
            "+00:00:00.000",
        ] {
            assert_eq!(
                FrameTimestamp::from_str(text),
                Err(FrameTimestampParseError::InvalidFormat),
                "{text:?}"
            );
        }
        for text in ["00:60:00.000", "00:00:60.000"] {
            assert_eq!(
                FrameTimestamp::from_str(text),
                Err(FrameTimestampParseError::ComponentOutOfRange)
            );
        }
        assert_eq!(
            FrameTimestamp::from_str("999999999999999999999:00:00.000"),
            Err(FrameTimestampParseError::Overflow)
        );
    }

    #[test]
    fn timestamp_filename_component_is_windows_safe_and_keeps_milliseconds() {
        let timestamp = FrameTimestamp::from_str("01:02:03.004").unwrap();
        assert_eq!(timestamp.filename_component(), "01-02-03-004");
    }

    #[test]
    fn resolved_positions_preserve_fractional_frame_identity_without_float_rounding() {
        let time_base = FrameTimeBase::new(1, 30_000).unwrap();
        let first = ResolvedFramePosition::new(1_001, time_base).unwrap();
        let second = ResolvedFramePosition::new(2_002, time_base).unwrap();

        assert_eq!(first.ffmpeg_timestamp(), "0.033366666");
        assert_eq!(second.ffmpeg_timestamp(), "0.066733333");
        assert_eq!(first.compare(second), Ordering::Less);
        assert_eq!(first.display_timestamp().milliseconds(), 33);
        assert_ne!(first, second);
    }

    #[test]
    fn requested_and_resolved_positions_keep_distinct_command_precision() {
        let requested = FramePosition::from(FrameTimestamp::from_milliseconds(33));
        let resolved = FramePosition::from(
            ResolvedFramePosition::new(1_001, FrameTimeBase::new(1, 30_000).unwrap()).unwrap(),
        );

        assert_eq!(requested.ffmpeg_timestamp(), "00:00:00.033");
        assert_eq!(resolved.ffmpeg_timestamp(), "0.033366666");
        assert_eq!(requested.display_timestamp(), resolved.display_timestamp());
    }
}
