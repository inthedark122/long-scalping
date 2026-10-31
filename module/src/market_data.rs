use crate::detection::mrc;
use crate::model::{Candle, IndicatorSnapshot, MrcBands, TimeframeSnapshot};
use lte_strategy_bridge::abi::ModuleKline;
use lte_strategy_bridge::kline_access::KlineHistory;
use std::collections::{BTreeMap, VecDeque};
use ta::indicators::{
    AverageTrueRange, ExponentialMovingAverage, Maximum, MeanReversionChannel, Minimum,
};
use ta::{Close, High, Low, Next};

pub const TF_1M: i32 = 60;
pub const TF_5M: i32 = 300;
pub const MRC_PERIOD: usize = 200;
pub const VOLUME_PERIOD: usize = 50;
pub const ATR_PERIOD: usize = 14;
pub const KLINE_HISTORY_COUNT: u32 = MRC_PERIOD as u32;

/// Request enough raw bars to warm the MRC calculation. The shorter volume
/// EMA and rolling extrema are calculated from the same kline history.
pub fn kline_requirements() -> BTreeMap<i32, u32> {
    BTreeMap::from([(TF_1M, KLINE_HISTORY_COUNT), (TF_5M, KLINE_HISTORY_COUNT)])
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrcHistorySample {
    pub time_ms: u64,
    pub close: f64,
    pub lines: [f64; 9],
}

struct TaCandle {
    high: f64,
    low: f64,
    close: f64,
}

impl High for TaCandle {
    fn high(&self) -> f64 {
        self.high
    }
}

impl Low for TaCandle {
    fn low(&self) -> f64 {
        self.low
    }
}

impl Close for TaCandle {
    fn close(&self) -> f64 {
        self.close
    }
}

struct CalculatedCandle {
    snapshot: TimeframeSnapshot,
    mrc_history: MrcHistorySample,
}

fn calculate_timeframe(
    rows: Option<&VecDeque<ModuleKline>>,
    timeframe: i32,
) -> Result<Vec<CalculatedCandle>, String> {
    let rows = rows.ok_or_else(|| format!("missing-kline-history tf={timeframe}"))?;
    if rows.is_empty() {
        return Err(format!("missing-kline-history tf={timeframe}"));
    }
    if rows.len() < KLINE_HISTORY_COUNT as usize {
        return Err(format!(
            "insufficient-kline-history tf={timeframe} rows={} required={KLINE_HISTORY_COUNT}",
            rows.len()
        ));
    }

    let mut channel = MeanReversionChannel::new(MRC_PERIOD);
    let mut atr = AverageTrueRange::new(ATR_PERIOD)
        .map_err(|error| format!("atr-init tf={timeframe} error={error}"))?;
    let mut volume_ema = ExponentialMovingAverage::new(VOLUME_PERIOD)
        .map_err(|error| format!("volume-ema-init tf={timeframe} error={error}"))?;
    let mut buy_sell_atr = AverageTrueRange::new(VOLUME_PERIOD)
        .map_err(|error| format!("buy-sell-atr-init tf={timeframe} error={error}"))?;
    let mut min_gain = Minimum::new(VOLUME_PERIOD)
        .map_err(|error| format!("volume-min-init tf={timeframe} error={error}"))?;
    let mut max_gain = Maximum::new(VOLUME_PERIOD)
        .map_err(|error| format!("volume-max-init tf={timeframe} error={error}"))?;
    let mut min_buy_sell_gain = Minimum::new(VOLUME_PERIOD)
        .map_err(|error| format!("buy-sell-min-init tf={timeframe} error={error}"))?;
    let mut max_buy_sell_gain = Maximum::new(VOLUME_PERIOD)
        .map_err(|error| format!("buy-sell-max-init tf={timeframe} error={error}"))?;

    let mut previous_middle = None;
    let mut calculated = Vec::with_capacity(rows.len());
    for row in rows {
        let ta_candle = TaCandle {
            high: row.high,
            low: row.low,
            close: row.close,
        };
        let channel_output = channel.next(&ta_candle);
        let atr = atr.next(&ta_candle);
        let volume = row.qty_buy + row.qty_sell;
        let average_volume = volume_ema.next(volume);
        let gain = if average_volume == 0.0 {
            0.0
        } else {
            (volume - average_volume) / average_volume
        };
        let buy_sell_delta = row.qty_buy - row.qty_sell;
        let buy_sell_atr = buy_sell_atr.next(buy_sell_delta);
        let buy_sell_gain = if buy_sell_atr > 0.0 {
            buy_sell_delta / buy_sell_atr
        } else {
            0.0
        };

        let bands = MrcBands {
            down_big: channel_output.down_big,
            down_outer: channel_output.down_outer,
            down_small: channel_output.down_small,
            down_inner: channel_output.down_inner,
            mean_line: channel_output.middle,
            up_inner: channel_output.up_inner,
            up_small: channel_output.up_small,
            up_outer: channel_output.up_outer,
            up_big: channel_output.up_big,
            current_cross: 0.0,
            mean_line_slope: previous_middle
                .map(|previous: f64| channel_output.middle - previous)
                .unwrap_or_default(),
        };
        previous_middle = Some(channel_output.middle);

        let volume_gain_min = min_gain.next(gain);
        let volume_gain_max = max_gain.next(gain);
        // Keep the matching rolling flow extrema warm as well, even though
        // current strategy decisions use only BuySellGain.
        let _ = min_buy_sell_gain.next(buy_sell_gain);
        let _ = max_buy_sell_gain.next(buy_sell_gain);
        let candle = Candle {
            open: row.open,
            high: row.high,
            low: row.low,
            close: row.close,
        };
        calculated.push(CalculatedCandle {
            snapshot: TimeframeSnapshot {
                candle,
                mrc: bands,
                atr,
                mrc_signal: mrc::analyze(&bands, row.close),
                volume_gain: gain,
                volume_gain_min,
                volume_gain_max,
                buy_sell_gain,
                // NTPS is not a kline input and this module does not request
                // platform-provided indicator values.
                ntps: 0,
            },
            mrc_history: MrcHistorySample {
                time_ms: row
                    .time
                    .saturating_add(i64::from(timeframe).saturating_mul(1_000))
                    .max(0) as u64,
                close: row.close,
                lines: bands.lines(),
            },
        });
    }
    Ok(calculated)
}

pub fn build_indicator_snapshot(history: &KlineHistory) -> Result<IndicatorSnapshot, String> {
    let tf1m = calculate_timeframe(history.get(TF_1M), TF_1M)?
        .pop()
        .ok_or_else(|| "missing-kline-history tf=60".to_string())?
        .snapshot;
    let tf5m = calculate_timeframe(history.get(TF_5M), TF_5M)?
        .pop()
        .ok_or_else(|| "missing-kline-history tf=300".to_string())?
        .snapshot;
    Ok(IndicatorSnapshot {
        tf1m,
        tf5m,
        ..Default::default()
    })
}

pub fn build_mrc_history(history: &KlineHistory) -> Result<Vec<MrcHistorySample>, String> {
    Ok(calculate_timeframe(history.get(TF_1M), TF_1M)?
        .into_iter()
        .map(|row| row.mrc_history)
        .collect())
}

pub fn render_kline_history_inventory(history: &KlineHistory) -> String {
    let count = |timeframe| history.get(timeframe).map_or(0, VecDeque::len);
    format!(
        "klines=60:{}/{} 300:{}/{}",
        count(TF_1M),
        KLINE_HISTORY_COUNT,
        count(TF_5M),
        KLINE_HISTORY_COUNT
    )
}

pub fn validate_indicator_snapshot(snapshot: &IndicatorSnapshot) -> Result<(), String> {
    let mut problems = Vec::new();

    for (label, tf) in [("1m", &snapshot.tf1m), ("5m", &snapshot.tf5m)] {
        let mut missing = Vec::new();
        if !tf.candle.close.is_finite() || tf.candle.close <= 0.0 {
            missing.push("close");
        }
        if !tf.mrc.range().is_finite() || tf.mrc.range() <= 0.0 {
            missing.push("mrc_range");
        }
        if !tf.mrc.ordered() {
            missing.push("mrc_order");
        }
        if !missing.is_empty() {
            problems.push(format!(
                "{} missing={} close={:.6} mean={:.6}",
                label,
                missing.join(","),
                tf.candle.close,
                tf.mrc.mean_line,
            ));
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("invalid-ta-snapshot {}", problems.join(" | ")))
    }
}
