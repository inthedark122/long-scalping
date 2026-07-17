use crate::model::{MarketState, RegimeSnapshot, TimeframeSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneMConfirmation {
    Aligned,
    Neutral,
    Tracked,
    Countertrend,
}

fn classify(tf: &TimeframeSnapshot) -> MarketState {
    let price_above_frozen_anchor = tf.dp_zee1 > 0.0 && tf.candle.close >= tf.dp_zee1;
    let price_below_frozen_anchor = tf.dp_zee1 > 0.0 && tf.candle.close <= tf.dp_zee1;
    let zigzag_allows_long = tf.zz_dir >= 0;
    let zigzag_allows_short = tf.zz_dir <= 0;

    if tf.mrc_signal.bullish_structure && price_above_frozen_anchor && zigzag_allows_long {
        MarketState::Long
    } else if tf.mrc_signal.bearish_structure && price_below_frozen_anchor && zigzag_allows_short {
        MarketState::Short
    } else {
        MarketState::Sideways
    }
}

pub fn classify_confirmation(confirmed: MarketState, tf1m: &TimeframeSnapshot) -> OneMConfirmation {
    let tf1m_direction = classify(tf1m);
    match (confirmed, tf1m_direction) {
        (MarketState::Long, MarketState::Long) => OneMConfirmation::Aligned,
        (MarketState::Short, MarketState::Short) => OneMConfirmation::Aligned,
        (MarketState::Sideways, MarketState::Long | MarketState::Short) => {
            OneMConfirmation::Tracked
        }
        (MarketState::Long | MarketState::Short, MarketState::Sideways) => {
            OneMConfirmation::Neutral
        }
        (MarketState::Long, MarketState::Short) | (MarketState::Short, MarketState::Long) => {
            OneMConfirmation::Countertrend
        }
        (MarketState::Sideways, MarketState::Sideways) => OneMConfirmation::Neutral,
    }
}

pub fn classify_regime(
    tf5: &TimeframeSnapshot,
    tf15: &TimeframeSnapshot,
    tf1h: &TimeframeSnapshot,
) -> RegimeSnapshot {
    let primary = classify(tf5);
    let fallback = classify(tf15);
    let context_1h = classify(tf1h);

    let fallback_used = primary == MarketState::Sideways && fallback != MarketState::Sideways;
    let provisional = if fallback_used { fallback } else { primary };

    RegimeSnapshot {
        primary,
        fallback,
        confirmed: provisional,
        fallback_used,
        context_1h,
        blocked_by_context: false,
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
            mrc_signal: crate::detection::mrc::analyze(
                &MrcBands {
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
                close,
            ),
            dp_dir: dp,
            zz_dir: zz,
            dp_zee: close - 1.0,
            dp_zee1: close - 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn regime_becomes_long_short_or_sideways() {
        let long = classify_regime(
            &tf(1, 0, 1.0, 111.0, 100.0),
            &tf(1, 0, 1.0, 111.0, 100.0),
            &tf(1, 0, 1.0, 111.0, 100.0),
        );
        assert_eq!(long.confirmed, MarketState::Long);

        let short = classify_regime(
            &TimeframeSnapshot {
                dp_zee1: 90.0,
                ..tf(-1, -1, -1.0, 89.0, 100.0)
            },
            &TimeframeSnapshot {
                dp_zee1: 90.0,
                ..tf(-1, -1, -1.0, 89.0, 100.0)
            },
            &TimeframeSnapshot {
                dp_zee1: 90.0,
                ..tf(-1, -1, -1.0, 89.0, 100.0)
            },
        );
        assert_eq!(short.confirmed, MarketState::Short);

        let sideways = classify_regime(
            &tf(1, -1, 0.0, 100.0, 100.0),
            &tf(1, -1, 0.0, 100.0, 100.0),
            &tf(1, -1, 0.0, 100.0, 100.0),
        );
        assert_eq!(sideways.confirmed, MarketState::Sideways);
    }

    #[test]
    fn fallback_resolves_ambiguous_primary() {
        let regime = classify_regime(
            &tf(1, -1, 0.0, 100.0, 100.0),
            &tf(1, 0, 1.0, 111.0, 100.0),
            &tf(1, 0, 1.0, 111.0, 100.0),
        );
        assert!(regime.fallback_used);
        assert_eq!(regime.confirmed, MarketState::Long);
    }

    #[test]
    fn confirmation_tracks_sideways_regime() {
        let conf = classify_confirmation(MarketState::Sideways, &tf(1, 0, 1.0, 111.0, 100.0));
        assert_eq!(conf, OneMConfirmation::Tracked);
    }

    #[test]
    fn confirmation_marks_alignment_and_neutral() {
        let aligned = classify_confirmation(MarketState::Long, &tf(1, 1, 1.0, 111.0, 100.0));
        assert_eq!(aligned, OneMConfirmation::Aligned);

        let neutral = classify_confirmation(MarketState::Short, &tf(1, 0, 1.0, 111.0, 100.0));
        assert_eq!(neutral, OneMConfirmation::Neutral);
    }
}
