use crate::detection::levels;
use crate::model::{IndicatorSnapshot, PositionState};
use lte_strategy_bridge::abi::{Direction, ModuleUpdatePosition};

const TP_MIN_PCT: f64 = 1.0;
const TP_MAX_PCT: f64 = 3.0;
const SL_MIN_PCT: f64 = 0.45;
const SL_MAX_PCT: f64 = 1.20;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExitPlan {
    pub take_profit: f64,
    pub stop_loss: f64,
    pub tp_reliable: bool,
}

pub fn plan(snapshot: &IndicatorSnapshot, price: f64, side: PositionState) -> Option<ExitPlan> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    let levels = levels::structural_levels(snapshot);
    let basis = price;

    match side {
        PositionState::Long => {
            let mut tp = levels
                .nearest_above(price)
                .unwrap_or(basis * (1.0 + TP_MIN_PCT / 100.0));
            let mut sl = levels
                .nearest_below(price)
                .unwrap_or(basis * (1.0 - SL_MIN_PCT / 100.0));

            tp = tp.min(basis * (1.0 + TP_MAX_PCT / 100.0));
            sl = sl.max(basis * (1.0 - SL_MAX_PCT / 100.0));

            if tp <= price {
                tp = basis * (1.0 + TP_MIN_PCT / 100.0);
            }
            if sl >= price {
                sl = basis * (1.0 - SL_MIN_PCT / 100.0);
            }

            let tp_gap_pct = if basis > 0.0 {
                (tp - basis).abs() / basis * 100.0
            } else {
                0.0
            };

            Some(ExitPlan {
                take_profit: tp,
                stop_loss: sl,
                tp_reliable: tp_gap_pct >= TP_MIN_PCT,
            })
        }
        PositionState::Short => {
            let mut tp = levels
                .nearest_below(price)
                .unwrap_or(basis * (1.0 - TP_MIN_PCT / 100.0));
            let mut sl = levels
                .nearest_above(price)
                .unwrap_or(basis * (1.0 + SL_MIN_PCT / 100.0));

            tp = tp.max(basis * (1.0 - TP_MAX_PCT / 100.0));
            sl = sl.min(basis * (1.0 + SL_MAX_PCT / 100.0));

            if tp >= price {
                tp = basis * (1.0 - TP_MIN_PCT / 100.0);
            }
            if sl <= price {
                sl = basis * (1.0 + SL_MIN_PCT / 100.0);
            }

            let tp_gap_pct = if basis > 0.0 {
                (basis - tp).abs() / basis * 100.0
            } else {
                0.0
            };

            Some(ExitPlan {
                take_profit: tp,
                stop_loss: sl,
                tp_reliable: tp_gap_pct >= TP_MIN_PCT,
            })
        }
        PositionState::Flat => None,
    }
}

pub fn update_position(side: PositionState, plan: ExitPlan) -> ModuleUpdatePosition {
    ModuleUpdatePosition {
        direction: match side {
            PositionState::Long => Direction::Long,
            PositionState::Short => Direction::Short,
            PositionState::Flat => Direction::Both,
        },
        enter_price: None,
        take_profit: Some(plan.take_profit),
        stop_loss: Some(plan.stop_loss),
    }
}

pub fn describe(plan: Option<ExitPlan>) -> String {
    match plan {
        Some(plan) => format!(
            "tp={:.4} sl={:.4} tp_note={}",
            plan.take_profit,
            plan.stop_loss,
            if plan.tp_reliable { "reliable" } else { "tight" }
        ),
        None => "tp=n/a sl=n/a".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, TimeframeSnapshot};

    fn tf(close: f64) -> TimeframeSnapshot {
        TimeframeSnapshot {
            candle: Candle {
                open: close,
                high: close + 1.0,
                low: close - 1.0,
                close,
            },
            mrc: MrcBands {
                down_big: 80.0,
                down_outer: 85.0,
                down_small: 90.0,
                down_inner: 95.0,
                mean_line: 100.0,
                up_inner: 105.0,
                up_small: 110.0,
                up_outer: 115.0,
                up_big: 120.0,
                current_cross: 0.0,
                mean_line_slope: 0.0,
            },
            dp_dir: 0,
            zz_dir: 0,
            ..Default::default()
        }
    }

    #[test]
    fn long_plan_prefers_structural_levels() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            fast_candle: None,
            signal: Default::default(),
        };
        let plan = plan(&snapshot, 100.0, PositionState::Long).unwrap();
        assert!(plan.take_profit > 100.0);
        assert!(plan.stop_loss < 100.0);
    }

    #[test]
    fn short_plan_prefers_structural_levels() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            fast_candle: None,
            signal: Default::default(),
        };
        let plan = plan(&snapshot, 100.0, PositionState::Short).unwrap();
        assert!(plan.take_profit < 100.0);
        assert!(plan.stop_loss > 100.0);
    }
}
