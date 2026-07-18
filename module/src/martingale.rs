use crate::exit::{ExitPlan, TakeProfit};
use crate::model::{EntryReason, IndicatorSnapshot, MarketState, PositionState};
use crate::orders::{limit_order, market_open};
use crate::state::EngineState;
use crate::stop_loss;
use lte_strategy_bridge::abi::{Direction, ModuleOpenPosition, ModulePlaceOrder};

pub const MIN_STEP_PCT: f64 = 0.7;
pub const MAX_STEP_PCT: f64 = 1.1;
pub const MIN_LEVELS: usize = 6;
pub const EXTRA_LEVELS: usize = 11;
pub const MAX_LEVELS: usize = MIN_LEVELS + EXTRA_LEVELS;
pub const TP_RATIO: f64 = 0.8;
pub const MIN_TP_PCT: f64 = 0.35;
pub const MAX_TP_PCT: f64 = 1.1;
pub const STOP_BREAK_MULTIPLIER: f64 = 1.25;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LadderProfile {
    pub levels: usize,
    pub range_pct: f64,
    pub step_pct: f64,
    pub take_profit_pct: f64,
}

impl LadderProfile {
    pub fn from_range(range_pct: f64) -> Option<Self> {
        if !range_pct.is_finite() || range_pct <= 0.0 {
            return None;
        }

        let levels = ((range_pct / MAX_STEP_PCT).ceil() as usize + 1).clamp(MIN_LEVELS, MAX_LEVELS);
        let step_pct = (range_pct / (levels.saturating_sub(1).max(1) as f64))
            .clamp(MIN_STEP_PCT, MAX_STEP_PCT);
        let take_profit_pct = (step_pct * TP_RATIO).clamp(MIN_TP_PCT, MAX_TP_PCT);

        Some(Self {
            levels,
            range_pct,
            step_pct,
            take_profit_pct,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MartingaleEntryPlan {
    pub market_state: MarketState,
    pub profile: LadderProfile,
    pub base_price: f64,
    pub open_position: ModuleOpenPosition,
    pub stop_loss: f64,
    pub orders: Vec<ModulePlaceOrder>,
    pub notes: Vec<String>,
}

fn direction(side: MarketState) -> Option<Direction> {
    match side {
        MarketState::Long => Some(Direction::Long),
        MarketState::Short => Some(Direction::Short),
        MarketState::Sideways => None,
    }
}

fn clamp_price(side: MarketState, price: f64) -> f64 {
    match side {
        MarketState::Long => price.max(f64::EPSILON),
        MarketState::Short => price.max(f64::EPSILON),
        MarketState::Sideways => price.max(f64::EPSILON),
    }
}

pub fn ladder_mark(level: usize) -> String {
    format!("extra-{}", level)
}

pub fn parse_ladder_mark(mark: Option<&str>) -> Option<usize> {
    let mark = mark?.strip_prefix("extra-")?;
    let level = mark.parse::<usize>().ok()?;
    (level > 0).then_some(level)
}

pub fn compute_profile(base_price: f64, range: f64) -> Option<LadderProfile> {
    if !base_price.is_finite() || base_price <= 0.0 {
        return None;
    }
    let range_pct = (range.abs() / base_price) * 100.0;
    LadderProfile::from_range(range_pct)
}

pub fn entry_plan(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    side: MarketState,
    entry_reason: Option<&EntryReason>,
    total_amount: f64,
) -> Option<MartingaleEntryPlan> {
    let side_direction = direction(side)?;
    let base_price = entry_reason
        .map(|reason| reason.price())
        .filter(|price| price.is_finite() && *price > 0.0)
        .unwrap_or(snapshot.tf1m.candle.close);
    let martingale_range = snapshot.tf1m.candle.high - snapshot.tf1m.candle.low;
    let profile = compute_profile(base_price, martingale_range)?;
    let total_amount = total_amount.max(0.0);
    if total_amount <= 0.0 {
        return None;
    }

    let step = base_price * (profile.step_pct / 100.0);
    let levels = profile.levels.max(1);
    let ladder_orders = 2usize.min(levels.saturating_sub(1));
    if ladder_orders == 0 {
        return None;
    }
    let base_amount = total_amount / 4.0;
    let open_position = market_open(side_direction, base_amount, base_price, "martingale-open");

    let mut orders = Vec::with_capacity(ladder_orders);
    for level in 1..=ladder_orders {
        if let Some(order) = ladder_order(side, profile, base_price, base_amount, level) {
            orders.push(order);
        }
    }

    let support_break = stop_loss::support_break_level(snapshot, to_position(side), entry_reason)
        .unwrap_or(match side {
            MarketState::Long => snapshot.tf1m.mrc.down_big,
            MarketState::Short => snapshot.tf1m.mrc.up_big,
            MarketState::Sideways => return None,
        });
    let ladder_range = step * ladder_orders as f64;
    let range_stop = ladder_range * STOP_BREAK_MULTIPLIER;
    let stop_loss = match side {
        MarketState::Long => (support_break - range_stop).max(f64::EPSILON),
        MarketState::Short => (support_break + range_stop).max(f64::EPSILON),
        MarketState::Sideways => return None,
    };

    let notes = vec![
        "martingale-entry".to_string(),
        "immediate-open".to_string(),
        format!("mode={}", state.cfg.trade_mode.as_str()),
        format!("levels={}", profile.levels),
        format!("range={:.4}", profile.range_pct),
        format!("step={:.2}%", profile.step_pct),
        format!("tp={:.2}%", profile.take_profit_pct),
        format!("base={:.4}", base_price),
        format!("sl={:.4}", stop_loss),
        format!("ladder-orders={}", orders.len()),
    ];

    Some(MartingaleEntryPlan {
        market_state: side,
        profile,
        base_price,
        open_position,
        stop_loss,
        orders,
        notes,
    })
}

pub fn ladder_order(
    side: MarketState,
    profile: LadderProfile,
    base_price: f64,
    base_amount: f64,
    level: usize,
) -> Option<ModulePlaceOrder> {
    let side_direction = direction(side)?;
    if level == 0 || level > profile.levels {
        return None;
    }

    let step = base_price * (profile.step_pct / 100.0);
    let price = match side {
        MarketState::Long => clamp_price(side, base_price - step * level as f64),
        MarketState::Short => clamp_price(side, base_price + step * level as f64),
        MarketState::Sideways => return None,
    };
    let amount = base_amount * level as f64;
    Some(limit_order(
        side_direction,
        amount,
        price,
        &ladder_mark(level),
        Some(&format!("martingale-entry:nearest-{}", level)),
    ))
}

fn to_position(side: MarketState) -> PositionState {
    match side {
        MarketState::Long => PositionState::Long,
        MarketState::Short => PositionState::Short,
        MarketState::Sideways => PositionState::Flat,
    }
}

pub fn exit_plan(
    snapshot: &IndicatorSnapshot,
    side: PositionState,
    entry_price: f64,
    entry_reason: Option<&EntryReason>,
    martingale_profile: Option<LadderProfile>,
) -> Option<ExitPlan> {
    if !entry_price.is_finite() || entry_price <= 0.0 {
        return None;
    }

    if let Some(EntryReason::SidewayScalp {
        price,
        block_low,
        block_high,
        ..
    }) = entry_reason
    {
        if !price.is_finite() || !block_low.is_finite() || !block_high.is_finite() {
            return None;
        }
        if block_high <= block_low {
            return None;
        }

        let width = (block_high - block_low).abs();
        let buffer = (width * 0.10).max((*price * 0.0005).max(f64::EPSILON));
        let (take_profit, stop_loss) = match side {
            PositionState::Long => (
                (*block_high).max(entry_price + buffer),
                (*block_low - buffer).max(f64::EPSILON),
            ),
            PositionState::Short => ((*block_low).max(f64::EPSILON), *block_high + buffer),
            PositionState::Flat => return None,
        };

        return Some(ExitPlan {
            take_profit: TakeProfit::Sideway { price: take_profit },
            stop_loss,
            tp_reliable: true,
        });
    }

    let profile = martingale_profile.or_else(|| {
        compute_profile(
            entry_price,
            snapshot.tf1m.candle.high - snapshot.tf1m.candle.low,
        )
    })?;
    let tp_distance = entry_price * (profile.take_profit_pct / 100.0);
    let break_price =
        stop_loss::support_break_level(snapshot, side, entry_reason).unwrap_or(match side {
            PositionState::Long => snapshot.tf1m.mrc.down_big,
            PositionState::Short => snapshot.tf1m.mrc.up_big,
            PositionState::Flat => return None,
        });
    let ladder_range = entry_price * (profile.range_pct / 100.0);
    let stop_distance = ladder_range * STOP_BREAK_MULTIPLIER;
    let (take_profit, stop_loss) = match side {
        PositionState::Long => (
            (entry_price + tp_distance).max(f64::EPSILON),
            (break_price - stop_distance).max(f64::EPSILON),
        ),
        PositionState::Short => (
            (entry_price - tp_distance).max(f64::EPSILON),
            (break_price + stop_distance).max(f64::EPSILON),
        ),
        PositionState::Flat => return None,
    };

    Some(ExitPlan {
        take_profit: TakeProfit::Martingale { price: take_profit },
        stop_loss,
        tp_reliable: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, TimeframeSnapshot, TradeMode};

    fn snapshot(close: f64, range: f64) -> IndicatorSnapshot {
        IndicatorSnapshot {
            tf1m: TimeframeSnapshot {
                candle: Candle {
                    open: close,
                    high: close + range * 0.5,
                    low: close - range * 0.5,
                    close,
                },
                mrc: MrcBands {
                    down_big: close - range,
                    down_outer: close - range * 0.75,
                    down_small: close - range * 0.5,
                    down_inner: close - range * 0.25,
                    mean_line: close,
                    up_inner: close + range * 0.25,
                    up_small: close + range * 0.5,
                    up_outer: close + range * 0.75,
                    up_big: close + range,
                    current_cross: 0.0,
                    mean_line_slope: 1.0,
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn profile_clamps_step_and_tp() {
        let profile = LadderProfile::from_range(12.0).unwrap();
        assert!(profile.levels >= MIN_LEVELS);
        assert!(profile.step_pct >= MIN_STEP_PCT);
        assert!(profile.step_pct <= MAX_STEP_PCT);
        assert!(profile.take_profit_pct >= MIN_TP_PCT);
        assert!(profile.take_profit_pct <= MAX_TP_PCT);
    }

    #[test]
    fn entry_plan_creates_two_nearest_orders_with_double_weight() {
        let mut state = EngineState::default();
        state.cfg.trade_mode = TradeMode::Martingale;
        let plan = entry_plan(
            &state,
            &snapshot(100.0, 8.0),
            MarketState::Long,
            Some(&EntryReason::SugInfoScalp { price: 100.0 }),
            30.0,
        )
        .unwrap();
        assert_eq!(plan.orders.len(), 2);
        assert_eq!(
            plan.open_position.order_type,
            lte_strategy_bridge::abi::ModuleOrderType::Market
        );
        assert_eq!(plan.open_position.enter_price, Some(100.0));
        let open = plan.open_position.amount.unwrap();
        let first = plan.orders[0].amount.unwrap();
        let second = plan.orders[1].amount.unwrap();
        assert!((first - open).abs() < f64::EPSILON);
        assert!((second - first * 2.0).abs() < f64::EPSILON);
        assert!(plan.orders[0].enter_price < plan.open_position.enter_price.unwrap());
        assert!(plan.stop_loss < 100.0);
    }

    #[test]
    fn exit_plan_sets_tp_above_price_and_sl_below_break() {
        let plan = exit_plan(
            &snapshot(100.0, 8.0),
            PositionState::Long,
            100.0,
            Some(&EntryReason::SugInfoScalp { price: 100.0 }),
            None,
        )
        .unwrap();
        assert!(plan.take_profit.price() > 100.0);
        assert!(plan.stop_loss < 100.0);
        assert!(matches!(plan.take_profit, TakeProfit::Martingale { .. }));
    }

    #[test]
    fn sideway_entry_uses_block_edge_stop() {
        let plan = exit_plan(
            &snapshot(100.0, 8.0),
            PositionState::Long,
            100.0,
            Some(&EntryReason::SidewayScalp {
                price: 100.0,
                block_low: 99.2,
                block_high: 100.8,
                source: "LineCross".to_string(),
            }),
            None,
        )
        .unwrap();
        assert!(matches!(plan.take_profit, TakeProfit::Sideway { .. }));
        assert!(plan.take_profit.price() >= 100.8);
        assert!(plan.stop_loss < 99.2);
        assert!(plan.stop_loss > 98.0);
    }
}
