use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::detection::mrc_pattern_classification::{MrcPatternKind, MrcTransitionKind};
use crate::detection::mrc_stair_entry::ReversalState;
use crate::model::{Candle, MrcBands};
use crate::strategies::{EntryDecision, EntryPlan, StrategyKind};
use lte_strategy_bridge::abi::Direction;

const STRONG_CANDLE_BODY_RATIO: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setup {
    pub direction: Direction,
    pub entry_price: f64,
    pub take_profit: Option<f64>,
    pub stop_loss: f64,
    pub mrc_lines: [f64; 9],
    pub entry_line: u8,
    pub target_line: Option<u8>,
    pub stop_line: u8,
    pub reason: &'static str,
    pub target_source: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Wait(&'static str),
    Enter(Setup),
}

pub fn as_entry_decision(decision: Decision) -> EntryDecision {
    match decision {
        Decision::Wait(reason) => EntryDecision::Wait {
            strategy: StrategyKind::Trending,
            reason,
        },
        Decision::Enter(setup) => {
            let target_line = setup
                .target_line
                .map(|line| crate::detection::mrc_crossing::LINE_LABELS[line as usize])
                .unwrap_or("none");
            let target_price = setup
                .take_profit
                .map(|price| format!("{price:.8}"))
                .unwrap_or_else(|| "trailing-stop".to_string());
            let mrc_lines = crate::detection::mrc_crossing::LINE_LABELS
                .iter()
                .zip(setup.mrc_lines)
                .map(|(name, price)| format!("{name}@{price:.8}"))
                .collect::<Vec<_>>()
                .join(",");
            let note = format!(
                "strategy=trending entry={} signal_price={:.8} entry_line={}@{:.8} entry_line_source=nearest-to-signal-price mrc_lines=[{}] target_line={} target_source={} target={} stop_line={} stop={:.8}",
                setup.reason,
                setup.entry_price,
                crate::detection::mrc_crossing::LINE_LABELS[setup.entry_line as usize],
                setup.mrc_lines[setup.entry_line as usize],
                mrc_lines,
                target_line,
                setup.target_source,
                target_price,
                crate::detection::mrc_crossing::LINE_LABELS[setup.stop_line as usize],
                setup.stop_loss,
            );
            EntryDecision::Enter(EntryPlan {
                strategy: StrategyKind::Trending,
                direction: setup.direction,
                entry_price: setup.entry_price,
                take_profit: setup.take_profit,
                stop_loss: setup.stop_loss,
                reason: setup.reason,
                note,
                entry_type: None,
            })
        }
    }
}

pub fn evaluate(
    analysis: Option<&MrcCrossAnalysis>,
    detected_trend_direction: Option<i8>,
    mrc: MrcBands,
    candle: Candle,
    market_price: f64,
) -> Decision {
    let Some(lines) = valid_lines(mrc) else {
        return Decision::Wait("invalid-mrc-lines");
    };
    if !market_price.is_finite() || market_price <= 0.0 {
        return Decision::Wait("invalid-market-price");
    }

    if let Some((direction, reason)) = reversal_setup(analysis, mrc, market_price) {
        if let Some(setup) = build_setup(analysis, lines, market_price, direction, reason) {
            return Decision::Enter(setup);
        }
    }

    let trend_direction = detected_trend_direction
        .filter(|direction| matches!(direction, 1 | -1))
        .or_else(|| analysis.and_then(pattern_trend_direction));
    let candle_direction = strong_candle_direction(candle);
    let direction = trend_direction.filter(|direction| Some(*direction) == candle_direction);
    let Some(direction) = direction else {
        return Decision::Wait("no-confirmed-mrc-trend-entry");
    };
    let reason = if direction > 0 {
        "uptrend-confirmation"
    } else {
        "downtrend-confirmation"
    };
    build_setup(analysis, lines, market_price, direction, reason)
        .map(Decision::Enter)
        .unwrap_or(Decision::Wait("mrc-exit-line-unavailable"))
}

fn pattern_trend_direction(analysis: &MrcCrossAnalysis) -> Option<i8> {
    match analysis.pattern_kind? {
        MrcPatternKind::StairsUp => Some(1),
        MrcPatternKind::StairsDown => Some(-1),
        MrcPatternKind::TrendingCycle if matches!(analysis.trade_direction, 1 | -1) => {
            Some(analysis.trade_direction)
        }
        MrcPatternKind::FastBreakReturn | MrcPatternKind::RangeCycleSideway => None,
        MrcPatternKind::TrendingCycle => None,
    }
}

fn reversal_setup(
    analysis: Option<&MrcCrossAnalysis>,
    mrc: MrcBands,
    market_price: f64,
) -> Option<(i8, &'static str)> {
    let analysis = analysis?;
    let direction = match (analysis.transition, analysis.reversal_state) {
        (Some(MrcTransitionKind::Reversal), _) => analysis.trade_direction,
        (_, ReversalState::Confirmed) => analysis.trade_direction,
        (_, ReversalState::Potential) => -analysis.trade_direction,
        (_, ReversalState::None) => return None,
    };
    if !matches!(direction, 1 | -1) {
        return None;
    }

    let is_at_bottom = market_price <= mrc.mean_line;
    let is_at_top = market_price >= mrc.mean_line;
    if (direction > 0 && is_at_bottom) || (direction < 0 && is_at_top) {
        let reason = if analysis.reversal_state == ReversalState::Confirmed
            || analysis.transition == Some(MrcTransitionKind::Reversal)
        {
            "mrc-confirmed-reversal"
        } else {
            "mrc-potential-reversal"
        };
        Some((direction, reason))
    } else {
        None
    }
}

fn strong_candle_direction(candle: Candle) -> Option<i8> {
    let range = candle.high - candle.low;
    let body = candle.close - candle.open;
    if !range.is_finite()
        || range <= 0.0
        || !body.is_finite()
        || body.abs() / range < STRONG_CANDLE_BODY_RATIO
    {
        return None;
    }
    match body.total_cmp(&0.0) {
        std::cmp::Ordering::Greater => Some(1),
        std::cmp::Ordering::Less => Some(-1),
        std::cmp::Ordering::Equal => None,
    }
}

fn build_setup(
    analysis: Option<&MrcCrossAnalysis>,
    lines: [f64; 9],
    entry_price: f64,
    direction: i8,
    reason: &'static str,
) -> Option<Setup> {
    let target = target_line(analysis, lines, entry_price, direction);
    let (entry_line, _) = nearest_line_to_price(lines, entry_price)?;
    let (stop_line, stop_loss) = outer_band_stop(lines, entry_price, direction)?;
    Some(Setup {
        direction: if direction > 0 {
            Direction::Long
        } else {
            Direction::Short
        },
        entry_price,
        take_profit: target.map(|(_, price)| price),
        stop_loss,
        mrc_lines: lines,
        entry_line,
        target_line: target.map(|(line, _)| line),
        stop_line,
        reason,
        target_source: if target.is_some() {
            "mrc-history"
        } else {
            "trailing-stop"
        },
    })
}

fn target_line(
    analysis: Option<&MrcCrossAnalysis>,
    lines: [f64; 9],
    entry_price: f64,
    direction: i8,
) -> Option<(u8, f64)> {
    analysis
        .filter(|analysis| analysis.trade_direction == direction)
        .and_then(|analysis| analysis.trade_target_line)
        .and_then(|line| line_price(lines, line).map(|price| (line, price)))
        .filter(|(_, price)| is_favorable(*price, entry_price, direction))
}

fn nearest_line_to_price(lines: [f64; 9], price: f64) -> Option<(u8, f64)> {
    lines
        .iter()
        .copied()
        .enumerate()
        .min_by(|left, right| (left.1 - price).abs().total_cmp(&(right.1 - price).abs()))
        .map(|(line, price)| (line as u8, price))
}

fn outer_band_stop(lines: [f64; 9], entry_price: f64, direction: i8) -> Option<(u8, f64)> {
    let stop_line = if direction > 0 {
        0
    } else {
        (lines.len() - 1) as u8
    };
    let stop_price = line_price(lines, stop_line)?;
    let is_adverse = if direction > 0 {
        stop_price < entry_price
    } else {
        stop_price > entry_price
    };
    is_adverse.then_some((stop_line, stop_price))
}

fn line_price(lines: [f64; 9], line: u8) -> Option<f64> {
    lines.get(line as usize).copied()
}

fn is_favorable(price: f64, entry: f64, direction: i8) -> bool {
    if direction > 0 {
        price > entry
    } else {
        price < entry
    }
}

fn valid_lines(mrc: MrcBands) -> Option<[f64; 9]> {
    let lines = mrc.lines();
    (mrc.ordered() && lines.iter().all(|line| line.is_finite() && *line > 0.0)).then_some(lines)
}
