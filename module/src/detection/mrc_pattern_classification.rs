//! Stable classification vocabulary for MRC pattern research.
//!
//! These classifications describe local movement behavior. They are not
//! trading decisions by themselves and deliberately do not encode a fixed
//! history length or a rare, complete crossing path. A research observation
//! can match more than one classification while it is developing; for
//! example, an outward continuation can become a zigzag staircase and later
//! become a fast break-and-return.

use crate::state::MrcHistoryEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MrcPatternKind {
    /// An alternating upward zigzag whose successive upward legs penetrate
    /// farther through the MRC lines.
    StairsUp,
    /// An alternating downward zigzag whose successive downward legs
    /// penetrate farther through the MRC lines.
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
    pub const ALL: [Self; 7] = [
        Self::StairsUp,
        Self::StairsDown,
        Self::FastBreakReturn,
        Self::RangeCycleSideway,
        Self::TrendingCycle,
        Self::PullbackToInner,
        Self::OutwardContinuation,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::StairsUp => 0,
            Self::StairsDown => 1,
            Self::FastBreakReturn => 2,
            Self::RangeCycleSideway => 3,
            Self::TrendingCycle => 4,
            Self::PullbackToInner => 5,
            Self::OutwardContinuation => 6,
        }
    }

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

/// Lifecycle of a confirmed directional staircase.
///
/// A staircase can remain directionally valid while its current leg is
/// consolidating or pulling back. It is only invalidated when the opposing
/// leg breaks the protected swing that supported the latest confirmed
/// expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaircaseLifecycle {
    Progressing,
    Pullback,
    Stalled,
    Invalidated,
}

impl StaircaseLifecycle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Progressing => "progressing",
            Self::Pullback => "pullback",
            Self::Stalled => "stalled",
            Self::Invalidated => "invalidated",
        }
    }
}

/// Structural transition that produced the current staircase lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaircaseTransition {
    HigherExtreme,
    PullbackWithinStructure,
    AttemptingExtreme,
    ProtectedSwingBroken,
}

impl StaircaseTransition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HigherExtreme => "higher_extreme",
            Self::PullbackWithinStructure => "pullback_within_structure",
            Self::AttemptingExtreme => "attempting_extreme",
            Self::ProtectedSwingBroken => "protected_swing_broken",
        }
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

#[derive(Debug, Clone, PartialEq)]
pub struct MrcPatternSignal {
    pub kind: MrcPatternKind,
    pub source_line: u8,
    pub target_line: u8,
    pub direction: i8,
    /// The line that was crossed before returning to `source_line`.
    pub failed_break_line: Option<u8>,
    /// Direction of the original break, before the return.
    pub failed_break_direction: i8,
    pub steps: u32,
    pub elapsed_ms: u64,
    /// The line path supporting a staircase decision. Other pattern kinds
    /// leave this empty.
    pub sequence: Vec<u8>,
    /// Direction-adjusted net price movement from the beginning of the
    /// staircase to the current event, in percent.
    pub price_move_pct: f64,
    /// Maximum favorable price excursion from the staircase start, in
    /// percent.
    pub favorable_move_pct: f64,
    /// Maximum adverse price excursion from the staircase start, in percent.
    pub adverse_move_pct: f64,
    /// Current lifecycle for staircase patterns. Other patterns leave this
    /// unset.
    pub staircase_lifecycle: Option<StaircaseLifecycle>,
    /// Structural transition that produced the current staircase lifecycle.
    pub staircase_transition: Option<StaircaseTransition>,
    /// The last pullback line protected by the confirmed staircase direction.
    pub staircase_protected_line: Option<u8>,
    /// The furthest line reached by the latest confirmed directional leg.
    pub staircase_extreme_line: Option<u8>,
}

const MEAN_LINE: u8 = 4;
const LOWER_ZONE_MAX: u8 = 3;
const UPPER_ZONE_MIN: u8 = 5;
const MIN_CYCLE_EXTREME_VISITS: usize = 4;
const SIDEWAY_DRIFT_RATIO: f64 = 0.25;
const FAST_BREAK_RETURN_MIN_ELAPSED_MS: u64 = 5 * 60_000;
const FAST_BREAK_RETURN_MAX_ELAPSED_MS: u64 = 15 * 60_000;

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

fn stair_price_metrics(
    history: &[MrcHistoryEvent],
    start_index: usize,
    direction: i8,
) -> (Vec<u8>, f64, f64, f64) {
    let sequence = history[start_index..]
        .iter()
        .map(|event| event.line)
        .collect::<Vec<_>>();
    let Some(start) = history.get(start_index) else {
        return (sequence, 0.0, 0.0, 0.0);
    };
    let start_price = start.market_price;
    let Some(current) = history.last() else {
        return (sequence, 0.0, 0.0, 0.0);
    };
    if !matches!(direction, 1 | -1)
        || !start_price.is_finite()
        || start_price <= 0.0
        || !current.market_price.is_finite()
        || current.market_price <= 0.0
    {
        return (sequence, 0.0, 0.0, 0.0);
    }
    let prices = history[start_index..]
        .iter()
        .map(|event| event.market_price)
        .filter(|price| price.is_finite() && *price > 0.0)
        .collect::<Vec<_>>();
    let high = prices.iter().copied().fold(start_price, f64::max);
    let low = prices.iter().copied().fold(start_price, f64::min);
    let net = (current.market_price - start_price) / start_price * 100.0 * direction as f64;
    let favorable = if direction > 0 {
        (high - start_price) / start_price * 100.0
    } else {
        (start_price - low) / start_price * 100.0
    };
    let adverse = if direction > 0 {
        (start_price - low) / start_price * 100.0
    } else {
        (high - start_price) / start_price * 100.0
    };
    (sequence, net, favorable, adverse)
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
    let elapsed_ms = returned.now_ms.saturating_sub(source.now_ms);
    if !(FAST_BREAK_RETURN_MIN_ELAPSED_MS..=FAST_BREAK_RETURN_MAX_ELAPSED_MS).contains(&elapsed_ms)
    {
        return None;
    }
    let direction = line_direction(crossed.line, returned.line);
    Some(MrcPatternSignal {
        kind: MrcPatternKind::FastBreakReturn,
        source_line: returned.line,
        target_line: adjacent_line(returned.line, direction)?,
        direction,
        failed_break_line: Some(crossed.line),
        failed_break_direction: line_direction(source.line, crossed.line),
        steps: 1,
        elapsed_ms,
        sequence: Vec::new(),
        price_move_pct: 0.0,
        favorable_move_pct: 0.0,
        adverse_move_pct: 0.0,
        staircase_lifecycle: None,
        staircase_transition: None,
        staircase_protected_line: None,
        staircase_extreme_line: None,
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
        failed_break_line: None,
        failed_break_direction: 0,
        steps: 1,
        elapsed_ms: returned.now_ms.saturating_sub(outward.now_ms),
        sequence: Vec::new(),
        price_move_pct: 0.0,
        favorable_move_pct: 0.0,
        adverse_move_pct: 0.0,
        staircase_lifecycle: None,
        staircase_transition: None,
        staircase_protected_line: None,
        staircase_extreme_line: None,
    })
}

#[derive(Debug, Clone, Copy)]
struct ZigzagLeg {
    direction: i8,
    start_index: usize,
    start_line: u8,
    end_line: u8,
}

#[derive(Debug, Clone)]
struct StaircaseSegment {
    start_index: usize,
    trend_indices: Vec<usize>,
    first_span: u8,
    best_end_line: u8,
    protected_line: u8,
    last_progress_index: usize,
    expanded_after_first: bool,
}

impl StaircaseSegment {
    fn new(index: usize, leg: ZigzagLeg) -> Self {
        Self {
            start_index: leg.start_index,
            trend_indices: vec![index],
            first_span: leg.start_line.abs_diff(leg.end_line),
            best_end_line: leg.end_line,
            protected_line: leg.start_line,
            last_progress_index: index,
            expanded_after_first: false,
        }
    }

    fn observe(&mut self, index: usize, leg: ZigzagLeg, direction: i8) {
        if is_deeper(leg.end_line, self.best_end_line, direction) {
            self.best_end_line = leg.end_line;
            self.protected_line = leg.start_line;
            self.last_progress_index = index;
            self.expanded_after_first = true;
        }
        self.trend_indices.push(index);
    }

    fn confirmed(&self) -> bool {
        self.trend_indices.len() >= 2 && (self.first_span > 1 || self.expanded_after_first)
    }
}

fn is_deeper(line: u8, best: u8, direction: i8) -> bool {
    if direction > 0 {
        line > best
    } else {
        line < best
    }
}

fn breaks_protected(line: u8, protected: u8, direction: i8) -> bool {
    if direction > 0 {
        line < protected
    } else {
        line > protected
    }
}

/// Compress adjacent line crossings into directional legs.
///
/// A path such as `2 -> 3 -> 2 -> 3 -> 4` becomes:
///
/// ```text
/// up(2 -> 3), down(3 -> 2), up(2 -> 4)
/// ```
///
/// Repeated movement in the same direction belongs to one leg. A skipped or
/// repeated line breaks the local zigzag rather than being silently bridged.
fn zigzag_legs(history: &[MrcHistoryEvent]) -> Vec<ZigzagLeg> {
    let mut legs: Vec<ZigzagLeg> = Vec::new();
    for index in 1..history.len() {
        let previous = &history[index - 1];
        let current = &history[index];
        let direction = line_direction(previous.line, current.line);
        if direction == 0 || previous.line.abs_diff(current.line) != 1 {
            legs.clear();
            continue;
        }

        if let Some(last) = legs.last_mut() {
            if last.direction == direction {
                last.end_line = current.line;
                continue;
            }
        }

        legs.push(ZigzagLeg {
            direction,
            start_index: index - 1,
            start_line: previous.line,
            end_line: current.line,
        });
    }
    legs
}

/// Detect one directional staircase as a lifecycle of zigzag structure.
///
/// A failed attempt to make a new extreme is a stall, not a reversal. The
/// staircase remains valid while the opposing leg holds above the protected
/// swing (below it for a down staircase). Breaking that swing invalidates the
/// current segment. A new segment must then build two directional legs before
/// the pattern can become active again.
fn staircase_for_direction(
    history: &[MrcHistoryEvent],
    legs: &[ZigzagLeg],
    trend_direction: i8,
) -> Option<MrcPatternSignal> {
    if legs.len() < 3 {
        return None;
    }

    let current = history.last()?;
    let current_leg = legs.last()?;
    let trend_indices = legs
        .iter()
        .enumerate()
        .filter_map(|(index, leg)| (leg.direction == trend_direction).then_some(index))
        .collect::<Vec<_>>();
    let first_index = *trend_indices.first()?;
    let mut segment = StaircaseSegment::new(first_index, legs[first_index]);
    let mut invalidated_segment = None;
    let mut fresh_segment: Option<StaircaseSegment> = None;

    for (index, leg) in legs.iter().enumerate().skip(first_index + 1) {
        if leg.direction == trend_direction {
            if invalidated_segment.is_some() {
                if let Some(fresh) = fresh_segment.as_mut() {
                    fresh.observe(index, *leg, trend_direction);
                } else {
                    fresh_segment = Some(StaircaseSegment::new(index, *leg));
                }
                if fresh_segment
                    .as_ref()
                    .is_some_and(StaircaseSegment::confirmed)
                {
                    segment = fresh_segment.take()?;
                    invalidated_segment = None;
                }
            } else {
                segment.observe(index, *leg, trend_direction);
            }
        } else if invalidated_segment.is_none()
            && segment.confirmed()
            && breaks_protected(leg.end_line, segment.protected_line, trend_direction)
        {
            invalidated_segment = Some(segment.clone());
            fresh_segment = None;
        }
    }

    let (lifecycle, transition, reported_segment) = if let Some(segment) = invalidated_segment {
        (
            StaircaseLifecycle::Invalidated,
            StaircaseTransition::ProtectedSwingBroken,
            segment,
        )
    } else if !segment.confirmed() {
        return None;
    } else {
        let transition = if current_leg.direction == trend_direction {
            if segment.last_progress_index == legs.len() - 1 {
                StaircaseTransition::HigherExtreme
            } else {
                StaircaseTransition::AttemptingExtreme
            }
        } else {
            StaircaseTransition::PullbackWithinStructure
        };
        let lifecycle = match transition {
            StaircaseTransition::HigherExtreme => StaircaseLifecycle::Progressing,
            StaircaseTransition::AttemptingExtreme => StaircaseLifecycle::Stalled,
            StaircaseTransition::PullbackWithinStructure => StaircaseLifecycle::Pullback,
            StaircaseTransition::ProtectedSwingBroken => StaircaseLifecycle::Invalidated,
        };
        (lifecycle, transition, segment)
    };
    let start_index = reported_segment.start_index;
    let trend_indices = reported_segment.trend_indices;
    let (sequence, price_move_pct, favorable_move_pct, adverse_move_pct) =
        stair_price_metrics(history, start_index, trend_direction);
    let kind = if trend_direction > 0 {
        MrcPatternKind::StairsUp
    } else {
        MrcPatternKind::StairsDown
    };
    Some(MrcPatternSignal {
        kind,
        source_line: current.line,
        target_line: adjacent_line(current.line, trend_direction)?,
        direction: trend_direction,
        failed_break_line: None,
        failed_break_direction: 0,
        steps: trend_indices.len() as u32,
        elapsed_ms: current.now_ms.saturating_sub(history[start_index].now_ms),
        sequence,
        price_move_pct,
        favorable_move_pct,
        adverse_move_pct,
        staircase_lifecycle: Some(lifecycle),
        staircase_transition: Some(transition),
        staircase_protected_line: Some(reported_segment.protected_line),
        staircase_extreme_line: Some(reported_segment.best_end_line),
    })
}

fn staircases(history: &[MrcHistoryEvent]) -> Vec<MrcPatternSignal> {
    let legs = zigzag_legs(history);
    if legs.len() < 3 {
        return Vec::new();
    }
    [1_i8, -1_i8]
        .into_iter()
        .filter_map(|direction| staircase_for_direction(history, &legs, direction))
        .collect()
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
        failed_break_line: None,
        failed_break_direction: 0,
        steps: (visits.len() - 1) as u32,
        elapsed_ms: last.now_ms.saturating_sub(first.now_ms),
        sequence: Vec::new(),
        price_move_pct: 0.0,
        favorable_move_pct: 0.0,
        adverse_move_pct: 0.0,
        staircase_lifecycle: None,
        staircase_transition: None,
        staircase_protected_line: None,
        staircase_extreme_line: None,
    })
}

/// Classifies the current local movement and returns the direction/line that
/// the trading layer should follow. The precedence is intentional: a direct
/// return or pullback is more specific than a staircase or a broad cycle.
pub fn classify(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    classify_all(history).into_iter().next()
}

fn outward_continuation(history: &[MrcHistoryEvent]) -> Option<MrcPatternSignal> {
    if history.len() < 2 {
        return None;
    }
    let [source, current] = history.get(history.len() - 2..)? else {
        return None;
    };
    let direction = line_direction(source.line, current.line);
    if source.line.abs_diff(current.line) != 1 || direction == 0 || !movement_success(current) {
        return None;
    }
    Some(MrcPatternSignal {
        kind: MrcPatternKind::OutwardContinuation,
        source_line: current.line,
        target_line: adjacent_line(current.line, direction)?,
        direction,
        failed_break_line: None,
        failed_break_direction: 0,
        steps: 1,
        elapsed_ms: current.now_ms.saturating_sub(source.now_ms),
        sequence: Vec::new(),
        price_move_pct: 0.0,
        favorable_move_pct: 0.0,
        adverse_move_pct: 0.0,
        staircase_lifecycle: None,
        staircase_transition: None,
        staircase_protected_line: None,
        staircase_extreme_line: None,
    })
}

/// Evaluate every independent matcher. The result deliberately keeps
/// overlapping matches: a staircase can also be an outward continuation and
/// a trending cycle can coexist with a local pullback.
pub fn classify_all(history: &[MrcHistoryEvent]) -> Vec<MrcPatternSignal> {
    if history.len() < 2 {
        return Vec::new();
    }
    let mut matches = Vec::new();
    matches.extend(fast_break_return(history));
    matches.extend(pullback_to_inner(history));
    matches.extend(staircases(history));
    matches.extend(cycle(history));
    matches.extend(outward_continuation(history));
    matches
}

/// The complete, stable catalogue. Keep this list small and behavior based;
/// learned thresholds and outcome statistics belong in the external history
/// files under `data/mrc-pattern-history/`.
pub const MRC_PATTERN_DEFINITIONS: [MrcPatternDefinition; 7] = [
    MrcPatternDefinition {
        kind: MrcPatternKind::StairsUp,
        title: "MRC upward staircase",
        summary: "Price zigzags upward while successive upward legs penetrate deeper.",
        detection: "At least two upward line legs separated by pullbacks. A missed higher high is a stall; the structure ends only when the protected swing low breaks.",
        prediction: "Follow the upward direction while pullbacks hold the protected swing low; wait through stalled attempts and invalidate only after a structural break.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::StairsDown,
        title: "MRC downward staircase",
        summary: "Price zigzags downward while successive downward legs penetrate deeper.",
        detection: "At least two downward line legs separated by pullbacks. A missed lower low is a stall; the structure ends only when the protected swing high breaks.",
        prediction: "Follow the downward direction while pullbacks hold the protected swing high; wait through stalled attempts and invalidate only after a structural break.",
    },
    MrcPatternDefinition {
        kind: MrcPatternKind::FastBreakReturn,
        title: "Fast break and return",
        summary: "Price crosses one adjacent line and returns to the source line within 5-15 minutes.",
        detection: "A -> adjacent B -> A within an inclusive 5-15 minute window, with excursion outside A and the exact returned line.",
        prediction: "Use the exact A -> B -> A history to compare rejection away from B with reclaim through B; B is the learned support/resistance barrier.",
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
