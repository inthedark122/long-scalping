use crate::detection::levels;
use crate::model::{EntryReason, IndicatorSnapshot, PositionState, SugInfoSnapshot};
use crate::state::CascadeState;
use crate::stop_loss;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ExitSource {
    #[default]
    Structural,
    SugInfo,
    Movement,
    Sideway,
    Martingale,
}

impl ExitSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Structural => "structural",
            Self::SugInfo => "sug-info",
            Self::Movement => "movement",
            Self::Sideway => "sideway",
            Self::Martingale => "martingale",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExitPlan {
    pub take_profit: f64,
    pub stop_loss: f64,
    pub tp_reliable: bool,
    pub source: ExitSource,
    pub structural_take_profit: f64,
    pub structural_stop_loss: f64,
}

fn structural_plan(
    levels: &crate::detection::levels::LevelBook,
    price: f64,
    side: PositionState,
) -> Option<(f64, f64)> {
    match side {
        PositionState::Long => {
            let tp = levels.nearest_above(price)?;
            let sl = levels.nearest_below(price)?;
            Some((tp, sl))
        }
        PositionState::Short => {
            let tp = levels.nearest_below(price)?;
            let sl = levels.nearest_above(price)?;
            Some((tp, sl))
        }
        PositionState::Flat => None,
    }
}

fn clamp_take_profit(
    price: f64,
    side: PositionState,
    raw_tp: f64,
    structural_take_profit: f64,
) -> f64 {
    if !raw_tp.is_finite() || raw_tp <= 0.0 {
        return 0.0;
    }

    let structural_valid = structural_take_profit.is_finite() && structural_take_profit > 0.0;
    match side {
        PositionState::Long => {
            if structural_valid && structural_take_profit > price {
                raw_tp.min(structural_take_profit)
            } else {
                raw_tp
            }
        }
        PositionState::Short => {
            if structural_valid && structural_take_profit < price {
                raw_tp.max(structural_take_profit)
            } else {
                raw_tp
            }
        }
        PositionState::Flat => raw_tp,
    }
}

fn movement_plan(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    structural: Option<(f64, f64)>,
) -> Option<ExitPlan> {
    let tp_distance = stop_loss::projected_move_distance(snapshot, sug_snapshot, cascade)?;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    let (structural_take_profit, structural_stop_loss) = structural.unwrap_or((0.0, 0.0));
    let support_stop =
        stop_loss::support_break_level(snapshot, side, None).unwrap_or(structural_stop_loss);

    let plan = match side {
        PositionState::Long => ExitPlan {
            take_profit: clamp_take_profit(
                price,
                side,
                price + tp_distance,
                structural_take_profit,
            ),
            stop_loss: support_stop.max(f64::EPSILON),
            tp_reliable: true,
            source: ExitSource::Movement,
            structural_take_profit,
            structural_stop_loss,
        },
        PositionState::Short => ExitPlan {
            take_profit: clamp_take_profit(
                price,
                side,
                (price - tp_distance).max(f64::EPSILON),
                structural_take_profit,
            ),
            stop_loss: support_stop.max(f64::EPSILON),
            tp_reliable: true,
            source: ExitSource::Movement,
            structural_take_profit,
            structural_stop_loss,
        },
        PositionState::Flat => return None,
    };

    if plan.take_profit.is_finite()
        && plan.stop_loss.is_finite()
        && plan.take_profit > 0.0
        && plan.stop_loss > 0.0
    {
        Some(plan)
    } else {
        None
    }
}

fn sideway_plan(reason: &EntryReason, side: PositionState) -> Option<ExitPlan> {
    let EntryReason::SidewayScalp {
        price,
        block_low,
        block_high,
        ..
    } = reason
    else {
        return None;
    };

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
            (*block_high).max(*price + buffer),
            (*block_low - buffer).max(f64::EPSILON),
        ),
        PositionState::Short => ((*block_low).max(f64::EPSILON), *block_high + buffer),
        PositionState::Flat => return None,
    };

    Some(ExitPlan {
        take_profit,
        stop_loss,
        tp_reliable: true,
        source: ExitSource::Sideway,
        structural_take_profit: take_profit,
        structural_stop_loss: stop_loss,
    })
}

fn compute(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    entry_reason: Option<&EntryReason>,
) -> Option<ExitPlan> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    if let Some(reason) = entry_reason.and_then(|reason| sideway_plan(reason, side)) {
        return Some(reason);
    }

    let levels = levels::structural_levels(snapshot);
    let structural = structural_plan(&levels, price, side);

    if let Some(plan) = movement_plan(snapshot, sug_snapshot, cascade, price, side, structural) {
        return Some(plan);
    }

    if let Some(sug) = sug_snapshot.sug_candle {
        let sug_side = sug_snapshot.sug_direction;
        let side_dir = match side {
            PositionState::Long => 1,
            PositionState::Short => -1,
            PositionState::Flat => 0,
        };
        if sug_side == 0 || sug_side == side_dir {
            let range = (sug.high - sug.low).abs();
            if range > 0.0 {
                let plan = match side {
                    PositionState::Long => ExitPlan {
                        take_profit: if sug.close > price {
                            sug.close
                        } else {
                            sug.high.max(price + range * 0.25)
                        },
                        stop_loss: sug.low.min(price - range * 0.10),
                        tp_reliable: true,
                        source: ExitSource::SugInfo,
                        structural_take_profit: structural.map(|(tp, _)| tp).unwrap_or(0.0),
                        structural_stop_loss: structural.map(|(_, sl)| sl).unwrap_or(0.0),
                    },
                    PositionState::Short => ExitPlan {
                        take_profit: if sug.close < price {
                            sug.close
                        } else {
                            sug.low.min(price - range * 0.25)
                        },
                        stop_loss: sug.high.max(price + range * 0.10),
                        tp_reliable: true,
                        source: ExitSource::SugInfo,
                        structural_take_profit: structural.map(|(tp, _)| tp).unwrap_or(0.0),
                        structural_stop_loss: structural.map(|(_, sl)| sl).unwrap_or(0.0),
                    },
                    PositionState::Flat => ExitPlan::default(),
                };
                if plan.take_profit.is_finite()
                    && plan.stop_loss.is_finite()
                    && plan.take_profit > 0.0
                    && plan.stop_loss > 0.0
                {
                    return Some(plan);
                }
            }
        }
    }

    match side {
        PositionState::Long => {
            let Some(raw_tp) = levels.nearest_above(price) else {
                return None;
            };
            let Some(raw_sl) = levels.nearest_below(price) else {
                return None;
            };

            Some(ExitPlan {
                take_profit: raw_tp,
                stop_loss: raw_sl,
                tp_reliable: true,
                source: ExitSource::Structural,
                structural_take_profit: raw_tp,
                structural_stop_loss: raw_sl,
            })
        }
        PositionState::Short => {
            let Some(raw_tp) = levels.nearest_below(price) else {
                return None;
            };
            let Some(raw_sl) = levels.nearest_above(price) else {
                return None;
            };

            Some(ExitPlan {
                take_profit: raw_tp,
                stop_loss: raw_sl,
                tp_reliable: true,
                source: ExitSource::Structural,
                structural_take_profit: raw_tp,
                structural_stop_loss: raw_sl,
            })
        }
        PositionState::Flat => None,
    }
}

pub fn plan(
    snapshot: &IndicatorSnapshot,
    sug_snapshot: &SugInfoSnapshot,
    cascade: &CascadeState,
    price: f64,
    side: PositionState,
    entry_reason: Option<&EntryReason>,
) -> Option<ExitPlan> {
    compute(snapshot, sug_snapshot, cascade, price, side, entry_reason)
}

pub fn describe(plan: Option<ExitPlan>) -> String {
    match plan {
        Some(plan) => format!(
            "tp={:.4} sl={:.4} src={} struct_tp={:.4} struct_sl={:.4} tp_note={}",
            plan.take_profit,
            plan.stop_loss,
            plan.source.as_str(),
            plan.structural_take_profit,
            plan.structural_stop_loss,
            if plan.tp_reliable {
                "reliable"
            } else {
                "tight"
            }
        ),
        None => "tp=n/a sl=n/a".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, SugInfoSnapshot, TimeframeSnapshot};

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
            ..Default::default()
        };
        let sug = SugInfoSnapshot::default();
        let cascade = CascadeState {
            avg_size: 5.0,
            samples: 3,
            ..Default::default()
        };
        let plan = plan(&snapshot, &sug, &cascade, 100.0, PositionState::Long, None).unwrap();
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
            ..Default::default()
        };
        let sug = SugInfoSnapshot::default();
        let cascade = CascadeState {
            avg_size: 5.0,
            samples: 3,
            ..Default::default()
        };
        let plan = plan(&snapshot, &sug, &cascade, 100.0, PositionState::Short, None).unwrap();
        assert!(plan.take_profit < 100.0);
        assert!(plan.stop_loss > 100.0);
    }

    #[test]
    fn movement_tp_is_capped_by_structural_target() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        let sug = SugInfoSnapshot {
            fast_candle: Some(crate::model::FastCandleSnapshot {
                candle: Candle {
                    open: 100.0,
                    high: 120.0,
                    low: 80.0,
                    close: 110.0,
                },
                active: true,
            }),
            ..Default::default()
        };
        let cascade = CascadeState {
            avg_size: 50.0,
            samples: 5,
            ..Default::default()
        };

        let plan = plan(&snapshot, &sug, &cascade, 100.0, PositionState::Long, None).unwrap();
        assert!(plan.source == ExitSource::Movement || plan.source == ExitSource::Structural);
        assert!(plan.take_profit <= 105.0);
    }

    #[test]
    fn sideway_plan_uses_block_edges() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0),
            tf5m: tf(100.0),
            tf15m: tf(100.0),
            tf1h: tf(100.0),
            ..Default::default()
        };
        let sug = SugInfoSnapshot::default();
        let cascade = CascadeState::default();
        let reason = EntryReason::SidewayScalp {
            price: 99.0,
            block_low: 98.0,
            block_high: 102.0,
            started_at_ms: 1,
            source: "OrderBlock".to_string(),
        };
        let plan = plan(
            &snapshot,
            &sug,
            &cascade,
            99.0,
            PositionState::Long,
            Some(&reason),
        )
        .unwrap();
        assert_eq!(plan.source, ExitSource::Sideway);
        assert!(plan.take_profit >= 102.0);
        assert!(plan.stop_loss < 98.0);
    }
}
