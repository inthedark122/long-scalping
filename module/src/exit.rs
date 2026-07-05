use crate::model::{IndicatorSnapshot, PositionState};
use lte_strategy_bridge::abi::{Direction, ModuleUpdatePosition};

const TP_MIN_PCT: f64 = 1.0;
const TP_MAX_PCT: f64 = 3.0;
const SL_MIN_PCT: f64 = 0.45;
const SL_MAX_PCT: f64 = 1.20;
const LEVEL_TOLERANCE_PCT: f64 = 0.0015;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExitPlan {
    pub take_profit: f64,
    pub stop_loss: f64,
    pub tp_reliable: bool,
}

fn add_level(levels: &mut Vec<f64>, value: f64) {
    if !value.is_finite() || value <= 0.0 {
        return;
    }
    if levels.iter().any(|existing| (existing - value).abs() < f64::EPSILON) {
        return;
    }
    levels.push(value);
}

fn nearest_above(price: f64, levels: &[f64]) -> Option<f64> {
    levels
        .iter()
        .copied()
        .filter(|level| *level > price)
        .min_by(|a, b| a.total_cmp(b))
}

fn nearest_below(price: f64, levels: &[f64]) -> Option<f64> {
    levels
        .iter()
        .copied()
        .filter(|level| *level < price)
        .max_by(|a, b| a.total_cmp(b))
}

fn structural_levels(snapshot: &IndicatorSnapshot) -> (Vec<f64>, Vec<f64>) {
    let price = snapshot.tf1m.candle.close.max(0.0);
    let mut above = Vec::new();
    let mut below = Vec::new();

    for tf in [
        snapshot.tf1m,
        snapshot.tf5m,
        snapshot.tf15m,
        snapshot.tf1h,
    ] {
        add_level(&mut above, tf.mrc.up_small);
        add_level(&mut above, tf.mrc.up_inner);
        add_level(&mut above, tf.mrc.up_outer);
        add_level(&mut below, tf.mrc.down_small);
        add_level(&mut below, tf.mrc.down_inner);
        add_level(&mut below, tf.mrc.down_outer);
        add_level(&mut above, tf.dp_zee);
        add_level(&mut above, tf.dp_zee1);
        add_level(&mut above, tf.zz_sell_price);
        add_level(&mut above, tf.zz_break_price);
        add_level(&mut below, tf.dp_zee);
        add_level(&mut below, tf.dp_zee1);
        add_level(&mut below, tf.zz_buy_price);
        add_level(&mut below, tf.zz_break_price);

        add_level(&mut above, tf.candle.high);
        add_level(&mut below, tf.candle.low);
    }

    let tolerance = (price * LEVEL_TOLERANCE_PCT).max(0.01);
    above.sort_by(|a, b| a.total_cmp(b));
    below.sort_by(|a, b| b.total_cmp(a));

    // Merge very close levels so we do not overreact to tiny indicator noise.
    fn cluster(levels: &[f64], tolerance: f64) -> Vec<f64> {
        let mut out: Vec<f64> = Vec::new();
        for level in levels.iter().copied() {
            if let Some(last) = out.last_mut() {
                if (level - *last).abs() <= tolerance {
                    *last = (*last + level) / 2.0;
                    continue;
                }
            }
            out.push(level);
        }
        out
    }

    let above = cluster(&above, tolerance);
    let below = cluster(&below, tolerance);
    (above, below)
}

pub fn plan(snapshot: &IndicatorSnapshot, price: f64, side: PositionState) -> Option<ExitPlan> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    let (above_levels, below_levels) = structural_levels(snapshot);
    let basis = price;

    match side {
        PositionState::Long => {
            let mut tp = nearest_above(price, &above_levels).unwrap_or(basis * (1.0 + TP_MIN_PCT / 100.0));
            let mut sl = nearest_below(price, &below_levels).unwrap_or(basis * (1.0 - SL_MIN_PCT / 100.0));

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
            let mut tp = nearest_below(price, &below_levels).unwrap_or(basis * (1.0 - TP_MIN_PCT / 100.0));
            let mut sl = nearest_above(price, &above_levels).unwrap_or(basis * (1.0 + SL_MIN_PCT / 100.0));

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
        };
        let plan = plan(&snapshot, 100.0, PositionState::Short).unwrap();
        assert!(plan.take_profit < 100.0);
        assert!(plan.stop_loss > 100.0);
    }
}
