//! Stable classification vocabulary for MRC movement research.
//!
//! These classifications describe local movement behavior. They are not
//! trading decisions by themselves and deliberately do not encode a fixed
//! history length or a rare, complete crossing path. A research observation
//! can match more than one classification while it is developing; for
//! example, an outward continuation can become a staircase and later become
//! a fast break-and-return.

use crate::state::MrcHistoryEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MrcPatternKind {
    /// Consecutive upward MRC-line progress without a meaningful downward
    /// interruption, normally extending from the lower band toward the upper
    /// outer lines.
    StairsUp,
    /// Consecutive downward MRC-line progress without a meaningful upward
    /// interruption, normally extending from the upper band toward the lower
    /// outer lines.
    StairsDown,
    /// Price crosses one adjacent MRC line and returns to the source line in
    /// a short interval, marking the crossed line as a possible barrier.
    FastBreakReturn,
    /// Price alternates between lower and upper MRC zones while its net price
    /// drift stays small compared with the total distance travelled.
    RangeCycleSideway,
    /// Price alternates between lower and upper MRC zones, but the net drift
    /// is large enough that the repeated crossings are part of a trend.
    TrendingCycle,
    /// After an outward line movement, price crosses back toward the inner
    /// band before the expected continuation is completed.
    PullbackToInner,
    /// A local movement crosses the next line in the current direction and
    /// its favorable excursion is stronger than its adverse excursion.
    OutwardContinuation,
}

impl MrcPatternKind {
    /// Stable identifier used in research reports and Git-tracked history.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StairsUp => "STAIRS_UP",
            Self::StairsDown => "STAIRS_DOWN",
            Self::FastBreakReturn => "FAST_BREAK_RETURN",
            Self::RangeCycleSideway => "RANGE_CYCLE_SIDEWAY",
            Self::TrendingCycle => "TRENDING_CYCLE",
            Self::PullbackToInner => "PULLBACK_TO_INNER",
            Self::OutwardContinuation => "OUTWARD_CONTINUATION",
        }
    }

    /// Parses the stable identifier written by the research exporter.
    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "STAIRS_UP" => Self::StairsUp,
            "STAIRS_DOWN" => Self::StairsDown,
            "FAST_BREAK_RETURN" => Self::FastBreakReturn,
            "RANGE_CYCLE_SIDEWAY" => Self::RangeCycleSideway,
            "TRENDING_CYCLE" => Self::TrendingCycle,
            "PULLBACK_TO_INNER" => Self::PullbackToInner,
            "OUTWARD_CONTINUATION" => Self::OutwardContinuation,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MrcPatternDefinition {
    pub kind: MrcPatternKind,
    pub title: &'static str,
    pub summary: &'static str,
    pub detection: &'static str,
    pub prediction: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MrcPatternSignal {
    pub kind: MrcPatternKind,
    pub source_line: u8,
    pub target_line: u8,
    pub direction: i8,
    pub steps: u32,
    pub elapsed_ms: u64,
}

const MEAN_LINE: u8 = 4;
const LOWER_ZONE_MAX: u8 = 3;
const UPPER_ZONE_MIN: u8 = 5;
const MIN_CYCLE_EXTREME_VISITS: usize = 4;
const SIDEWAY_DRIFT_RATIO: f64 = 0.25;

fn adjacent_line(source: u8, direction: i8) -> Option<u8> {
    if direction > 0 {
        (source < 8).then_some(source + 1)
    } else if direction < 0 {
        source.checked_sub(1)
    } else {
        None
    }
}

fn line_direction(from: u8, to: u8) -> i8 {
    match to.cmp(&from) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

fn zone(line: u8) -> Option<bool> {
    if line <= LOWER_ZONE_MAX {
        Some(false)
    } else if line >= UPPER_ZONE_MIN {
        Some(true)
    } else {
        None
    }
}

fn distance_from_mean(line: u8) -> u8 {
    line.abs_diff(MEAN_LINE)
}

fn movement_success(event: &MrcHistoryEvent) -> bool {
    let Some(movement) = event.completed_movement else {
        return false;
    };
    let start = if movement.market_start_price > 0.0 {
        movement.market_start_price
    } else {
        movement.start_price
    };
    if start <= 0.0 || !start.is_finite() || movement.direction == 0 {
        return false;
    }
    let (favorable, adverse) = if movement.direction > 0 {
        (
            (movement.high - start) / start * 100.0,
            (movement.low - start) / start * 100.0,
        )
    } else {
        (
            (start - movement.low) / start * 100.0,
            (start - movement.high) / start * 100.0,
        )
    };
    favorable > adverse.abs()
}

fn fast_break_return(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    let [source, crossed, returned] = history.get(history.len().checked_sub(3)?..)? else {
        return None;
    };
    if source.line != returned.line || source.line.abs_diff(crossed.line) != 1 {
        return None;
    }
    let direction = line_direction(crossed.line, returned.line);
    Some(MrcPatternSignal {
        kind: MrcPatternKind::FastBreakReturn,
        source_line: returned.line,
        target_line: adjacent_line(returned.line, direction)?,
        direction,
        steps: 1,
        elapsed_ms: returned.now_ms.saturating_sub(source.now_ms),
    })
}

fn pullback_to_inner(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    let [source, outward, returned] = history.get(history.len().checked_sub(3)?..)? else {
        return None;
    };
    let outward_direction = line_direction(source.line, outward.line);
    let return_direction = line_direction(outward.line, returned.line);
    if outward.line.abs_diff(source.line) != 1
        || distance_from_mean(outward.line) <= distance_from_mean(source.line)
        || distance_from_mean(returned.line) >= distance_from_mean(outward.line)
        || returned.line == source.line
        || outward_direction == 0
        || return_direction == 0
    {
        return None;
    }
    Some(MrcPatternSignal {
        kind: MrcPatternKind::PullbackToInner,
        source_line: returned.line,
        target_line: adjacent_line(returned.line, return_direction)?,
        direction: return_direction,
        steps: 1,
        elapsed_ms: returned.now_ms.saturating_sub(outward.now_ms),
    })
}

fn staircase(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    if history.len() < 3 {
        return None;
    }
    let current = history.last()?;
    let mut start_index = history.len() - 1;
    let mut direction = 0;
    let mut steps = 0u32;
    while start_index > 0 {
        let previous = &history[start_index - 1];
        let delta = previous.line.abs_diff(history[start_index].line);
        let step_direction = line_direction(previous.line, history[start_index].line);
        if delta != 1 || step_direction == 0 || (direction != 0 && direction != step_direction) {
            break;
        }
        direction = step_direction;
        steps = steps.saturating_add(1);
        start_index -= 1;
    }
    if steps < 2 {
        return None;
    }
    let start = history.get(start_index)?;
    let kind = if direction > 0 {
        MrcPatternKind::StairsUp
    } else {
        MrcPatternKind::StairsDown
    };
    Some(MrcPatternSignal {
        kind,
        source_line: current.line,
        target_line: adjacent_line(current.line, direction)?,
        direction,
        steps,
        elapsed_ms: current.now_ms.saturating_sub(start.now_ms),
    })
}

fn cycle(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    let mut extreme_indices = Vec::new();
    for (index, event) in history.iter().enumerate() {
        if zone(event.line).is_some() {
            extreme_indices.push(index);
        }
    }
    let indices = extreme_indices.get(
        extreme_indices
            .len()
            .checked_sub(MIN_CYCLE_EXTREME_VISITS)?..,
    )?;
    let visits = indices
        .iter()
        .map(|index| zone(history[*index].line).map(|side| (side, *index)))
        .collect::<Option<Vec<_>>>()?;
    if visits.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    let first_index = visits.first()?.1;
    let last_index = visits.last()?.1;
    let first = history.get(first_index)?;
    let last = history.get(last_index)?;
    let prices = history[first_index..=last_index]
        .iter()
        .map(|event| event.market_price)
        .filter(|price| price.is_finite() && *price > 0.0)
        .collect::<Vec<_>>();
    if prices.is_empty() || first.market_price <= 0.0 || !first.market_price.is_finite() {
        return None;
    }
    let total_range = (prices.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - prices.iter().copied().fold(f64::INFINITY, f64::min))
        / first.market_price;
    let drift = (last.market_price - first.market_price).abs() / first.market_price;
    let kind = if total_range <= 0.0 || drift / total_range <= SIDEWAY_DRIFT_RATIO {
        MrcPatternKind::RangeCycleSideway
    } else {
        MrcPatternKind::TrendingCycle
    };
    let current = history.last()?;
    let direction = if kind == MrcPatternKind::TrendingCycle {
        line_direction(first.line, last.line).signum()
    } else if zone(current.line) == Some(false) {
        1
    } else if zone(current.line) == Some(true) {
        -1
    } else {
        0
    };
    Some(MrcPatternSignal {
        kind,
        source_line: current.line,
        target_line: if kind == MrcPatternKind::RangeCycleSideway {
            if direction > 0 {
                UPPER_ZONE_MIN
            } else {
                LOWER_ZONE_MAX
            }
        } else {
            adjacent_line(current.line, direction)?
        },
        direction,
        steps: (visits.len() - 1) as u32,
        elapsed_ms: last.now_ms.saturating_sub(first.now_ms),
    })
}

/// Classifies the current local movement and returns the direction/line that
/// the trading layer should follow. The precedence is intentional: a direct
/// return or pullback is more specific than a staircase or a broad cycle.
pub fn classify(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    if history.len() < 2 {
        return None;
    }
    fast_break_return(history)
        .or_else(|| pullback_to_inner(history))
        .or_else(|| staircase(history))
        .or_else(|| cycle(history))
        .or_else(|| {
            let [source, current] = history.get(history.len() - 2..)? else {
                return None;
            };
            let direction = line_direction(source.line, current.line);
            if source.line.abs_diff(current.line) != 1
                || direction == 0
                || !movement_success(current)
            {
                return None;
            }
            Some(MrcPatternSignal {
                kind: MrcPatternKind::OutwardContinuation,
                source_line: current.line,
                target_line: adjacent_line(current.line, direction)?,
                direction,
                steps: 1,
                elapsed_ms: current.now_ms.saturating_sub(source.now_ms),
            })
        })
}

/// The complete, stable catalogue. Keep this list small and behavior based;
/// learned thresholds and outcome statistics belong in the external history
/// files under `data/mrc-pattern-history/`.
pub const MRC_PATTERN_DEFINITIONS: [MrcPatternDefinition; 7] = [
    MrcPatternDefinition {
        kind: MrcPatternKind::StairsUp,
        title: "MRC upward staircase",
        summary: "Price advances through successive MRC lines toward the upper band.",
        detection: "A monotonic upward run of local line transitions, usually beginning at or below DownInner and reaching UpOuter or UpBig.",
        prediction: "Continue toward the next unbroken upper line while the favorable excursion remains stronger than the adverse excursion.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::StairsDown,
        title: "MRC downward staircase",
        summary: "Price declines through successive MRC lines toward the lower band.",
        detection: "A monotonic downward run of local line transitions, usually beginning at or above UpInner and reaching DownOuter or DownBig.",
        prediction: "Continue toward the next unbroken lower line while the favorable excursion remains stronger than the adverse excursion.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::FastBreakReturn,
        title: "Fast break and return",
        summary: "Price crosses one adjacent line and quickly returns to the source line.",
        detection: "A -> adjacent B -> A, measured by elapsed time, excursion outside A, and the exact returned line.",
        prediction: "Treat B as a possible resistance or support barrier and project movement away from the failed break.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::RangeCycleSideway,
        title: "Sideways range cycle",
        summary: "Price repeatedly travels between lower and upper MRC zones with limited net drift.",
        detection: "Alternating visits to the lower and upper zones, with price drift compared against the total travelled range.",
        prediction: "Expect another edge-to-edge movement and use the opposite zone as the likely destination.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::TrendingCycle,
        title: "Trending cycle",
        summary: "Price crosses lower and upper zones repeatedly while the whole range drifts directionally.",
        detection: "The same alternating zone behavior as a sideways cycle, but with materially larger net drift relative to total range.",
        prediction: "Prefer the drift direction and treat opposing edge touches as pullbacks instead of automatic reversals.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::PullbackToInner,
        title: "Pullback to inner line",
        summary: "After an outward movement, price returns toward an inner MRC line before continuation is confirmed.",
        detection: "An outward transition followed by an inward transition, including the returned line, elapsed time, and adverse excursion.",
        prediction: "Reduce continuation confidence and evaluate the returned inner line as a possible reversal or re-entry area.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::OutwardContinuation,
        title: "Outward continuation",
        summary: "A local movement crosses the next line in the current direction and produces favorable excursion.",
        detection: "A source line followed by its adjacent line in the same direction, with movement percentages and duration recorded.",
        prediction: "Use the next line as the immediate destination and feed repeated continuations into staircase learning.",
    },
];

pub fn definition(kind: MrcPatternKind) -> &'static MrcPatternDefinition {
    MRC_PATTERN_DEFINITIONS
        .iter()
        .find(|definition| definition.kind == kind)
        .expect("every MRC pattern kind has a definition")
}
