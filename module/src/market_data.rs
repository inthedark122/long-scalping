use crate::detection::mrc;
use crate::model::{Candle, IndicatorSnapshot, MrcBands, TimeframeSnapshot};
use lte_strategy_bridge::abi::{ModuleKline, ModuleKlineSnapshot};
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

#[derive(Clone)]
struct MrcCheckpoint {
    time: i64,
    channel: MeanReversionChannel,
    previous_middle: Option<f64>,
    latest: Option<MrcBands>,
    last_time: Option<i64>,
}

struct MrcTimeframe {
    channel: MeanReversionChannel,
    previous_middle: Option<f64>,
    latest: Option<MrcBands>,
    last_time: Option<i64>,
    history: VecDeque<MrcHistorySample>,
    checkpoints: VecDeque<MrcCheckpoint>,
}

impl Default for MrcTimeframe {
    fn default() -> Self {
        Self {
            channel: MeanReversionChannel::new(MRC_PERIOD),
            previous_middle: None,
            latest: None,
            last_time: None,
            history: VecDeque::new(),
            checkpoints: VecDeque::new(),
        }
    }
}

impl MrcTimeframe {
    fn initialize(
        &mut self,
        rows: Option<&VecDeque<ModuleKline>>,
        timeframe: i32,
    ) -> Result<(), String> {
        *self = Self::default();
        let rows = rows.ok_or_else(|| format!("missing-kline-history tf={timeframe}"))?;
        if rows.is_empty() {
            return Err(format!("missing-kline-history tf={timeframe}"));
        }
        for row in rows {
            self.process(row, timeframe);
        }
        Ok(())
    }

    fn update(
        &mut self,
        updates: Option<&[ModuleKline]>,
        retained_rows: Option<&VecDeque<ModuleKline>>,
        timeframe: i32,
    ) -> Result<(), String> {
        let Some(updates) = updates else {
            return Ok(());
        };

        let revised_time = updates
            .iter()
            .filter(|row| self.last_time.is_some_and(|last| row.time <= last))
            .map(|row| row.time)
            .min();

        if let Some(revised_time) = revised_time {
            let index = self
                .checkpoints
                .iter()
                .position(|checkpoint| checkpoint.time == revised_time)
                .ok_or_else(|| {
                    format!(
                        "mrc-update-outside-retained-history tf={timeframe} time={revised_time}"
                    )
                })?;
            let checkpoint = self.checkpoints[index].clone();
            self.channel = checkpoint.channel;
            self.previous_middle = checkpoint.previous_middle;
            self.latest = checkpoint.latest;
            self.last_time = checkpoint.last_time;
            self.checkpoints.truncate(index);
            self.history.truncate(index);

            let rows =
                retained_rows.ok_or_else(|| format!("missing-kline-history tf={timeframe}"))?;
            for row in rows.iter().filter(|row| row.time >= revised_time) {
                self.process(row, timeframe);
            }
            return Ok(());
        }

        for row in updates {
            if self.last_time.is_none_or(|last| row.time > last) {
                self.process(row, timeframe);
            }
        }
        Ok(())
    }

    fn process(&mut self, row: &ModuleKline, timeframe: i32) {
        self.checkpoints.push_back(MrcCheckpoint {
            time: row.time,
            channel: self.channel.clone(),
            previous_middle: self.previous_middle,
            latest: self.latest,
            last_time: self.last_time,
        });

        let output = self.channel.next(&TaCandle {
            high: row.high,
            low: row.low,
            close: row.close,
        });
        let bands = MrcBands {
            down_big: output.down_big,
            down_outer: output.down_outer,
            down_small: output.down_small,
            down_inner: output.down_inner,
            mean_line: output.middle,
            up_inner: output.up_inner,
            up_small: output.up_small,
            up_outer: output.up_outer,
            up_big: output.up_big,
            current_cross: 0.0,
            mean_line_slope: self
                .previous_middle
                .map(|previous| output.middle - previous)
                .unwrap_or_default(),
        };
        self.previous_middle = Some(output.middle);
        self.latest = Some(bands);
        self.last_time = Some(row.time);
        self.history.push_back(MrcHistorySample {
            time_ms: row
                .time
                .saturating_add(i64::from(timeframe).saturating_mul(1_000))
                .max(0) as u64,
            close: row.close,
            lines: bands.lines(),
        });

        while self.history.len() > KLINE_HISTORY_COUNT as usize {
            self.history.pop_front();
            self.checkpoints.pop_front();
        }
    }
}

/// Holds the existing ta-rs MRC filters between kline updates. It is runtime
/// calculation state and is deliberately omitted from the module output state.
#[derive(Default)]
pub struct MrcCalculation {
    one_minute: MrcTimeframe,
    five_minute: MrcTimeframe,
}

impl MrcCalculation {
    pub fn initialize(&mut self, history: &KlineHistory) -> Result<(), String> {
        *self = Self::default();
        self.one_minute.initialize(history.get(TF_1M), TF_1M)?;
        self.five_minute.initialize(history.get(TF_5M), TF_5M)
    }

    pub fn update(
        &mut self,
        updates: &ModuleKlineSnapshot,
        history: &KlineHistory,
    ) -> Result<(), String> {
        self.one_minute.update(
            updates.get(&TF_1M).map(Vec::as_slice),
            history.get(TF_1M),
            TF_1M,
        )?;
        self.five_minute.update(
            updates.get(&TF_5M).map(Vec::as_slice),
            history.get(TF_5M),
            TF_5M,
        )
    }

    pub fn latest_bands(&self, timeframe: i32) -> Option<MrcBands> {
        match timeframe {
            TF_1M => self.one_minute.latest,
            TF_5M => self.five_minute.latest,
            _ => None,
        }
    }

    fn one_minute_history(&self) -> Vec<MrcHistorySample> {
        self.one_minute.history.iter().copied().collect()
    }
}

fn calculate_timeframe(
    rows: Option<&VecDeque<ModuleKline>>,
    timeframe: i32,
    mrc_bands: MrcBands,
) -> Result<TimeframeSnapshot, String> {
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

    let mut latest = None;
    for row in rows {
        let ta_candle = TaCandle {
            high: row.high,
            low: row.low,
            close: row.close,
        };
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
        latest = Some(TimeframeSnapshot {
            candle,
            mrc: mrc_bands,
            atr,
            mrc_signal: mrc::analyze(&mrc_bands, row.close),
            volume_gain: gain,
            volume_gain_min,
            volume_gain_max,
            buy_sell_gain,
            // NTPS is not a kline input and this module does not request
            // platform-provided indicator values.
            ntps: 0,
        });
    }
    latest.ok_or_else(|| format!("missing-kline-history tf={timeframe}"))
}

pub fn build_indicator_snapshot(
    history: &KlineHistory,
    mrc_calculation: &MrcCalculation,
) -> Result<IndicatorSnapshot, String> {
    let tf1m_bands = mrc_calculation
        .latest_bands(TF_1M)
        .ok_or_else(|| "missing-mrc-calculation tf=60".to_string())?;
    let tf5m_bands = mrc_calculation
        .latest_bands(TF_5M)
        .ok_or_else(|| "missing-mrc-calculation tf=300".to_string())?;
    let tf1m = calculate_timeframe(history.get(TF_1M), TF_1M, tf1m_bands)?;
    let tf5m = calculate_timeframe(history.get(TF_5M), TF_5M, tf5m_bands)?;
    Ok(IndicatorSnapshot {
        tf1m,
        tf5m,
        ..Default::default()
    })
}

pub fn build_mrc_history(
    mrc_calculation: &MrcCalculation,
) -> Result<Vec<MrcHistorySample>, String> {
    let history = mrc_calculation.one_minute_history();
    if history.is_empty() {
        return Err("missing-mrc-calculation tf=60".to_string());
    }
    Ok(history)
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
