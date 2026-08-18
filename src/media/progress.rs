use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaProgress {
    pub processed: Option<Duration>,
    pub total: Option<Duration>,
    pub fraction: Option<f32>,
    pub frame: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MediaProgressParser {
    total: Option<Duration>,
    processed: Option<Duration>,
    frame: Option<u64>,
}

impl MediaProgressParser {
    pub fn new(total: Option<Duration>) -> Self {
        Self {
            total,
            processed: None,
            frame: None,
        }
    }

    pub fn push_line(&mut self, line: &str) -> Option<MediaProgress> {
        let (key, value) = line.trim_end_matches(['\r', '\n']).split_once('=')?;

        match key {
            "frame" => {
                if let Some(frame) = parse_nonnegative_u64(value) {
                    self.frame = Some(frame);
                }
            }
            // FFmpeg's historical out_time_ms key is also expressed in microseconds.
            "out_time_us" | "out_time_ms" => {
                if let Some(processed) = parse_microseconds(value) {
                    self.processed = Some(processed);
                }
            }
            "out_time" => {
                if let Some(processed) = parse_timestamp(value) {
                    self.processed = Some(processed);
                }
            }
            "progress" if matches!(value, "continue" | "end") => {
                return Some(self.snapshot());
            }
            _ => {}
        }

        None
    }

    fn snapshot(&self) -> MediaProgress {
        MediaProgress {
            processed: self.processed,
            total: self.total,
            fraction: progress_fraction(self.processed, self.total),
            frame: self.frame,
        }
    }
}

fn parse_nonnegative_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

fn parse_microseconds(value: &str) -> Option<Duration> {
    Some(Duration::from_micros(parse_nonnegative_u64(value)?))
}

fn parse_timestamp(value: &str) -> Option<Duration> {
    let (hours, rest) = value.split_once(':')?;
    let (minutes, seconds) = rest.split_once(':')?;

    let hours = parse_nonnegative_u64(hours)?;
    let minutes = parse_nonnegative_u64(minutes)?;
    if minutes >= 60 {
        return None;
    }

    let (whole_seconds, fractional) = match seconds.split_once('.') {
        Some(parts) => parts,
        None => (seconds, ""),
    };
    let whole_seconds = parse_nonnegative_u64(whole_seconds)?;
    if whole_seconds >= 60 {
        return None;
    }

    let micros = parse_fractional_microseconds(fractional)?;
    let total_seconds = hours
        .checked_mul(60)?
        .checked_add(minutes)?
        .checked_mul(60)?
        .checked_add(whole_seconds)?;

    Duration::from_secs(total_seconds).checked_add(Duration::from_micros(micros))
}

fn parse_fractional_microseconds(value: &str) -> Option<u64> {
    if value.len() > 6 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    if value.is_empty() {
        return Some(0);
    }

    let mut micros = value.parse::<u64>().ok()?;
    for _ in value.len()..6 {
        micros = micros.checked_mul(10)?;
    }
    Some(micros)
}

fn progress_fraction(processed: Option<Duration>, total: Option<Duration>) -> Option<f32> {
    let processed = processed?;
    let total = total?;
    if total.is_zero() {
        return None;
    }

    Some((processed.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{MediaProgress, MediaProgressParser};
    use crate::media::process::{MediaProcessFailure, MediaProcessState};

    #[test]
    fn parses_normal_time_progress() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        assert_eq!(parser.push_line("out_time_us=2500000"), None);
        assert_eq!(
            parser.push_line("progress=continue"),
            Some(MediaProgress {
                processed: Some(Duration::from_millis(2_500)),
                total: Some(Duration::from_secs(10)),
                fraction: Some(0.25),
                frame: None,
            })
        );
    }

    #[test]
    fn parses_frame_count() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        parser.push_line("frame=240");
        let progress = parser.push_line("progress=continue").unwrap();

        assert_eq!(progress.frame, Some(240));
    }

    #[test]
    fn missing_total_duration_leaves_fraction_absent() {
        let mut parser = MediaProgressParser::new(None);

        parser.push_line("out_time=00:00:04.500000");
        let progress = parser.push_line("progress=continue").unwrap();

        assert_eq!(progress.processed, Some(Duration::from_millis(4_500)));
        assert_eq!(progress.total, None);
        assert_eq!(progress.fraction, None);
    }

    #[test]
    fn malformed_or_impossible_fields_are_ignored() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        parser.push_line("frame=12");
        parser.push_line("frame=-1");
        parser.push_line("out_time_us=not-a-number");
        parser.push_line("out_time=00:75:99.000000");
        parser.push_line("unknown_key=123");
        let progress = parser.push_line("progress=continue").unwrap();

        assert_eq!(progress.frame, Some(12));
        assert_eq!(progress.processed, None);
        assert_eq!(progress.fraction, None);
    }

    #[test]
    fn repeated_values_keep_the_latest_valid_value() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        parser.push_line("frame=10");
        parser.push_line("frame=20");
        parser.push_line("out_time_us=1000000");
        parser.push_line("out_time_us=3500000");
        let progress = parser.push_line("progress=continue").unwrap();

        assert_eq!(progress.frame, Some(20));
        assert_eq!(progress.processed, Some(Duration::from_millis(3_500)));
        assert_eq!(progress.fraction, Some(0.35));
    }

    #[test]
    fn out_of_range_fraction_is_clamped() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(2)));

        parser.push_line("out_time_us=5000000");
        let progress = parser.push_line("progress=continue").unwrap();

        assert_eq!(progress.processed, Some(Duration::from_secs(5)));
        assert_eq!(progress.fraction, Some(1.0));
    }

    #[test]
    fn progress_end_flushes_without_inventing_completion() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        parser.push_line("out_time_us=9000000");
        let progress = parser.push_line("progress=end").unwrap();

        assert_eq!(progress.processed, Some(Duration::from_secs(9)));
        assert_eq!(progress.fraction, Some(0.9));
    }

    #[test]
    fn apparent_hundred_percent_does_not_override_process_failure() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        parser.push_line("out_time_us=10000000");
        let progress = parser.push_line("progress=end").unwrap();
        let process_state = MediaProcessState::Failed(MediaProcessFailure::Exit(Some(1)));

        assert_eq!(progress.fraction, Some(1.0));
        assert!(matches!(process_state, MediaProcessState::Failed(_)));
    }

    #[test]
    fn historical_out_time_ms_is_treated_as_microseconds() {
        let mut parser = MediaProgressParser::new(Some(Duration::from_secs(10)));

        parser.push_line("out_time_ms=1250000");
        let progress = parser.push_line("progress=continue").unwrap();

        assert_eq!(progress.processed, Some(Duration::from_millis(1_250)));
    }
}
