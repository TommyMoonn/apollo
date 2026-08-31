use std::{fmt, path::PathBuf, str::FromStr};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub timestamp: FrameTimestamp,
    pub format: FrameCaptureFormat,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{FrameTimestamp, FrameTimestampParseError};

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
}
