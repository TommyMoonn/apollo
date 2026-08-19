use std::fmt;

use super::model::UpscaleTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpscaleSizingError {
    InvalidSourceDimensions {
        width: u32,
        height: u32,
    },
    TargetNotLarger {
        target: UpscaleTarget,
        source_width: u32,
        source_height: u32,
        boundary_width: u32,
        boundary_height: u32,
    },
}

impl fmt::Display for UpscaleSizingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceDimensions { width, height } => {
                write!(f, "source dimensions must be non-zero: {width}x{height}")
            }
            Self::TargetNotLarger {
                target,
                source_width,
                source_height,
                boundary_width,
                boundary_height,
            } => write!(
                f,
                "{target:?} boundary {boundary_width}x{boundary_height} is not larger than source {source_width}x{source_height}"
            ),
        }
    }
}

impl std::error::Error for UpscaleSizingError {}

/// Return the target box for the source orientation.
///
/// Landscape and square sources use the target's normal landscape boundary.
/// Portrait sources use the corresponding portrait boundary by swapping the
/// target edges. For example, the 4K boundary is `3840x2160` for landscape or
/// square media and `2160x3840` for portrait media.
pub const fn oriented_target_boundary(
    target: UpscaleTarget,
    source_width: u32,
    source_height: u32,
) -> OutputDimensions {
    let (width, height) = target.boundary();
    if source_height > source_width {
        OutputDimensions {
            width: height,
            height: width,
        }
    } else {
        OutputDimensions { width, height }
    }
}

/// Calculate the largest aspect-preserving output that fits the selected target.
///
/// The source is scaled continuously until one edge reaches its orientation-aware
/// target boundary. The limiting edge therefore uses the full selected boundary,
/// while the other edge is quantized to the nearest feasible even pixel count.
/// Quantization is constrained to remain at least as large as the source edge and
/// no larger than the selected boundary, minimizing the unavoidable ratio error
/// without cropping, padding, target overshoot, or a one-axis downscale.
pub fn calculate_output_dimensions(
    source_width: u32,
    source_height: u32,
    target: UpscaleTarget,
) -> Result<OutputDimensions, UpscaleSizingError> {
    if source_width == 0 || source_height == 0 {
        return Err(UpscaleSizingError::InvalidSourceDimensions {
            width: source_width,
            height: source_height,
        });
    }

    let boundary = oriented_target_boundary(target, source_width, source_height);
    if source_width >= boundary.width || source_height >= boundary.height {
        return Err(UpscaleSizingError::TargetNotLarger {
            target,
            source_width,
            source_height,
            boundary_width: boundary.width,
            boundary_height: boundary.height,
        });
    }

    let width_limited = u64::from(boundary.width) * u64::from(source_height)
        <= u64::from(boundary.height) * u64::from(source_width);

    let output = if width_limited {
        OutputDimensions {
            width: boundary.width,
            height: nearest_feasible_even_edge(
                source_height,
                boundary.width,
                source_width,
                boundary.height,
            ),
        }
    } else {
        OutputDimensions {
            width: nearest_feasible_even_edge(
                source_width,
                boundary.height,
                source_height,
                boundary.width,
            ),
            height: boundary.height,
        }
    };

    debug_assert!(output.width <= boundary.width);
    debug_assert!(output.height <= boundary.height);
    debug_assert!(output.width.is_multiple_of(2));
    debug_assert!(output.height.is_multiple_of(2));

    Ok(output)
}

fn nearest_feasible_even_edge(
    source_edge: u32,
    limiting_boundary_edge: u32,
    source_limit: u32,
    maximum_edge: u32,
) -> u32 {
    let numerator = u64::from(source_edge) * u64::from(limiting_boundary_edge);
    let denominator = u64::from(source_limit);
    let integer_floor =
        u32::try_from(numerator / denominator).expect("target boundaries fit within u32");
    let lower_even = integer_floor - (integer_floor % 2);
    let upper_even = lower_even.saturating_add(2);
    let minimum_even = source_edge + (source_edge % 2);

    [minimum_even, lower_even, upper_even, maximum_edge]
        .into_iter()
        .filter(|candidate| *candidate >= minimum_even && *candidate <= maximum_edge)
        .min_by_key(|candidate| {
            let scaled_candidate = u64::from(*candidate) * denominator;
            (scaled_candidate.abs_diff(numerator), *candidate)
        })
        .expect("a strictly larger even target boundary always provides a feasible edge")
}
