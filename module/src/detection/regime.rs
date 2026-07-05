use crate::detection::cascade::CascadeSignal;
use crate::detection::mrc::{analyze, MrcSignal};
use crate::model::{MarketState, RegimeSnapshot, TimeframeSnapshot};

fn classify(tf: &TimeframeSnapshot, mrc: MrcSignal, cascade: CascadeSignal) -> MarketState {
    let aligned = MarketState::from_dir_pair(tf.dp_dir, tf.zz_dir);
    match aligned {
        MarketState::Long if mrc.bullish_structure && cascade.dir >= 0 => MarketState::Long,
        MarketState::Short if mrc.bearish_structure && cascade.dir <= 0 => MarketState::Short,
        _ => MarketState::Sideways,
    }
}

pub fn classify_regime(
    tf5: &TimeframeSnapshot,
    tf15: &TimeframeSnapshot,
    tf1h: &TimeframeSnapshot,
    cas5: CascadeSignal,
    cas15: CascadeSignal,
    cas1h: CascadeSignal,
) -> RegimeSnapshot {
    let mrc5 = analyze(&tf5.mrc, tf5.candle.close);
    let mrc15 = analyze(&tf15.mrc, tf15.candle.close);
    let mrc1h = analyze(&tf1h.mrc, tf1h.candle.close);

    let primary = classify(tf5, mrc5, cas5);
    let fallback = classify(tf15, mrc15, cas15);
    let context_1h = classify(tf1h, mrc1h, cas1h);

    let fallback_used = primary == MarketState::Sideways && fallback != MarketState::Sideways;
    let provisional = if fallback_used { fallback } else { primary };
    let blocked_by_context = matches!(
        (provisional, context_1h),
        (MarketState::Long, MarketState::Short) | (MarketState::Short, MarketState::Long)
    );

    RegimeSnapshot {
        primary,
        confirmed: if blocked_by_context {
            MarketState::Sideways
        } else {
            provisional
        },
        fallback_used,
        context_1h,
        blocked_by_context,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, TimeframeSnapshot};

    fn tf(dp: i8, zz: i8, slope: f64, close: f64, mean: f64) -> TimeframeSnapshot {
        TimeframeSnapshot {
            candle: Candle {
                open: close,
                high: close,
                low: close,
                close,
            },
            mrc: MrcBands {
                down_big: 80.0,
                down_outer: 85.0,
                down_small: 90.0,
                down_inner: 95.0,
                mean_line: mean,
                up_inner: 105.0,
                up_small: 110.0,
                up_outer: 115.0,
                up_big: 120.0,
                current_cross: 1.0,
                mean_line_slope: slope,
            },
            dp_dir: dp,
            zz_dir: zz,
            ..Default::default()
        }
    }

    #[test]
    fn regime_becomes_long_short_or_sideways() {
        let long = classify_regime(
            &tf(1, 1, 1.0, 111.0, 100.0),
            &tf(1, 1, 1.0, 111.0, 100.0),
            &tf(1, 1, 1.0, 111.0, 100.0),
            CascadeSignal {
                dir: 1,
                strength: 1.0,
                stretched: false,
            },
            CascadeSignal {
                dir: 1,
                strength: 1.0,
                stretched: false,
            },
            CascadeSignal {
                dir: 1,
                strength: 1.0,
                stretched: false,
            },
        );
        assert_eq!(long.confirmed, MarketState::Long);

        let short = classify_regime(
            &tf(-1, -1, -1.0, 89.0, 100.0),
            &tf(-1, -1, -1.0, 89.0, 100.0),
            &tf(-1, -1, -1.0, 89.0, 100.0),
            CascadeSignal {
                dir: -1,
                strength: 1.0,
                stretched: false,
            },
            CascadeSignal {
                dir: -1,
                strength: 1.0,
                stretched: false,
            },
            CascadeSignal {
                dir: -1,
                strength: 1.0,
                stretched: false,
            },
        );
        assert_eq!(short.confirmed, MarketState::Short);

        let sideways = classify_regime(
            &tf(1, -1, 0.0, 100.0, 100.0),
            &tf(1, -1, 0.0, 100.0, 100.0),
            &tf(1, -1, 0.0, 100.0, 100.0),
            CascadeSignal::default(),
            CascadeSignal::default(),
            CascadeSignal::default(),
        );
        assert_eq!(sideways.confirmed, MarketState::Sideways);
    }

    #[test]
    fn fallback_resolves_ambiguous_primary() {
        let regime = classify_regime(
            &tf(1, -1, 0.0, 100.0, 100.0),
            &tf(1, 1, 1.0, 111.0, 100.0),
            &tf(1, 1, 1.0, 111.0, 100.0),
            CascadeSignal::default(),
            CascadeSignal {
                dir: 1,
                strength: 1.0,
                stretched: false,
            },
            CascadeSignal {
                dir: 1,
                strength: 1.0,
                stretched: false,
            },
        );
        assert!(regime.fallback_used);
        assert_eq!(regime.confirmed, MarketState::Long);
    }
}
