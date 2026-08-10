use crate::detection::mrc;
use crate::model::{
    Candle, FastCandleSnapshot, IndicatorSnapshot, MrcBands, SignalIndicatorSnapshot, SmcSnapshot,
};
use crate::model::{SugInfoSnapshot, TimeframeSnapshot};
use lte_strategy_bridge::abi::{Direction, ModuleIndicatorValue};
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};
use lte_strategy_bridge::sug_info::SuggestionInfo;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const TF_1M: TimeframeSec = TimeframeSec::Tf1m;
const TF_5M: TimeframeSec = TimeframeSec::Tf5m;
const TF_15M: TimeframeSec = TimeframeSec::Tf15m;
const TF_1H: TimeframeSec = TimeframeSec::Tf1h;

fn timeframe_from_secs(timeframe: i64) -> Option<TimeframeSec> {
    match timeframe {
        60 => Some(TF_1M),
        300 => Some(TF_5M),
        900 => Some(TF_15M),
        3600 => Some(TF_1H),
        _ => None,
    }
}

fn read_float(
    history: &IndicatorHistory,
    timeframe: TimeframeSec,
    indicator: IndicatorKey,
    field: IndicatorField,
    offset: usize,
) -> Result<f64, String> {
    match history.get_prev(
        IndicatorFieldKey {
            timeframe,
            indicator,
            field,
        },
        offset,
    ) {
        Some(ModuleIndicatorValue::Float(value)) => Ok(*value),
        Some(ModuleIndicatorValue::String(value)) => value.parse::<f64>().map_err(|_| {
            format!(
                "invalid-float tf={} indicator={} field={} offset={} value={}",
                timeframe.as_i64(),
                indicator.as_name(),
                field.as_name(),
                offset,
                value
            )
        }),
        Some(other) => Err(format!(
            "invalid-value tf={} indicator={} field={} offset={} value={:?}",
            timeframe.as_i64(),
            indicator.as_name(),
            field.as_name(),
            offset,
            other
        )),
        None => Err(format!(
            "missing-value tf={} indicator={} field={} offset={}",
            timeframe.as_i64(),
            indicator.as_name(),
            field.as_name(),
            offset
        )),
    }
}

fn read_direction(
    history: &IndicatorHistory,
    timeframe: TimeframeSec,
    indicator: IndicatorKey,
) -> Result<i8, String> {
    match history.get(IndicatorFieldKey {
        timeframe,
        indicator,
        field: IndicatorField::Direction,
    }) {
        Some(ModuleIndicatorValue::Direction(Direction::Long)) => Ok(1),
        Some(ModuleIndicatorValue::Direction(Direction::Short)) => Ok(-1),
        Some(ModuleIndicatorValue::Direction(Direction::Both)) => Ok(0),
        Some(value) => decode_direction(value).map_err(|_| {
            format!(
                "invalid-direction tf={} indicator={} field=Direction value={:?}",
                timeframe.as_i64(),
                indicator.as_name(),
                value
            )
        }),
        None => Err(format!(
            "missing-direction tf={} indicator={} field=Direction",
            timeframe.as_i64(),
            indicator.as_name()
        )),
    }
}

fn decode_direction(value: &ModuleIndicatorValue) -> Result<i8, ()> {
    match value {
        ModuleIndicatorValue::Direction(Direction::Long) => Ok(1),
        ModuleIndicatorValue::Direction(Direction::Short) => Ok(-1),
        ModuleIndicatorValue::Direction(Direction::Both) => Ok(0),
        ModuleIndicatorValue::String(value) if value.eq_ignore_ascii_case("LONG") => Ok(1),
        ModuleIndicatorValue::String(value) if value.eq_ignore_ascii_case("SHORT") => Ok(-1),
        ModuleIndicatorValue::String(value) if value.eq_ignore_ascii_case("BOTH") => Ok(0),
        _ => Err(()),
    }
}

pub(crate) type RawIndicators =
    BTreeMap<i64, Vec<HashMap<String, HashMap<String, ModuleIndicatorValue>>>>;

pub fn render_raw_indicator_inventory(raw: &RawIndicators) -> String {
    let mut frames = Vec::new();
    for (timeframe, candles) in raw {
        let mut indicators: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for candle in candles {
            for (indicator, fields) in candle {
                let entry = indicators.entry(indicator.clone()).or_default();
                for field in fields.keys() {
                    entry.insert(field.clone());
                }
            }
        }

        let mut rendered_indicators = Vec::new();
        for (indicator, fields) in indicators {
            let fields = fields.into_iter().collect::<Vec<_>>().join(",");
            rendered_indicators.push(format!("{}({})", indicator, fields));
        }

        frames.push(format!("{}[{}]", timeframe, rendered_indicators.join(";")));
    }

    if frames.is_empty() {
        "indicators=empty".to_string()
    } else {
        format!("indicators={}", frames.join(" | "))
    }
}

fn read_raw_float(
    indicators: &RawIndicators,
    timeframe: TimeframeSec,
    indicator: &str,
    field: &str,
) -> Result<f64, String> {
    let value = indicators
        .get(&timeframe.as_i64())
        .and_then(|candles| candles.first())
        .and_then(|by_ind| by_ind.get(indicator))
        .and_then(|fields| fields.get(field))
        .ok_or_else(|| {
            format!(
                "missing-raw tf={} indicator={} field={}",
                timeframe.as_i64(),
                indicator,
                field
            )
        })?;

    match value {
        ModuleIndicatorValue::Float(value) => Ok(*value),
        ModuleIndicatorValue::String(value) => value.parse::<f64>().map_err(|_| {
            format!(
                "invalid-raw-float tf={} indicator={} field={} value={}",
                timeframe.as_i64(),
                indicator,
                field,
                value
            )
        }),
        other => Err(format!(
            "invalid-raw-value tf={} indicator={} field={} value={:?}",
            timeframe.as_i64(),
            indicator,
            field,
            other
        )),
    }
}

fn read_optional_raw_float(
    indicators: &RawIndicators,
    timeframe: TimeframeSec,
    indicator: &str,
    field: &str,
) -> Result<Option<f64>, String> {
    let Some(value) = indicators
        .get(&timeframe.as_i64())
        .and_then(|candles| candles.first())
        .and_then(|by_ind| by_ind.get(indicator))
        .and_then(|fields| fields.get(field))
    else {
        return Ok(None);
    };

    match value {
        ModuleIndicatorValue::Float(value) => Ok(Some(*value)),
        ModuleIndicatorValue::String(value) => value.parse::<f64>().map(Some).map_err(|_| {
            format!(
                "invalid-raw-float tf={} indicator={} field={} value={}",
                timeframe.as_i64(),
                indicator,
                field,
                value
            )
        }),
        other => Err(format!(
            "invalid-raw-value tf={} indicator={} field={} value={:?}",
            timeframe.as_i64(),
            indicator,
            field,
            other
        )),
    }
}

fn read_optional_raw_direction(
    indicators: &RawIndicators,
    timeframe: TimeframeSec,
    indicator: &str,
    field: &str,
) -> Result<Option<i8>, String> {
    let Some(value) = indicators
        .get(&timeframe.as_i64())
        .and_then(|candles| candles.first())
        .and_then(|by_ind| by_ind.get(indicator))
        .and_then(|fields| fields.get(field))
    else {
        return Ok(None);
    };

    match decode_direction(value) {
        Ok(direction) => Ok(Some(direction)),
        Err(()) => Err(format!(
            "invalid-raw-direction tf={} indicator={} field={} value={:?}",
            timeframe.as_i64(),
            indicator,
            field,
            value
        )),
    }
}

fn read_raw_direction(
    indicators: &RawIndicators,
    timeframe: TimeframeSec,
    indicator: &str,
    field: &str,
) -> Result<i8, String> {
    let value = indicators
        .get(&timeframe.as_i64())
        .and_then(|candles| candles.first())
        .and_then(|by_ind| by_ind.get(indicator))
        .and_then(|fields| fields.get(field))
        .ok_or_else(|| {
            format!(
                "missing-raw-direction tf={} indicator={} field={}",
                timeframe.as_i64(),
                indicator,
                field
            )
        })?;

    match decode_direction(value) {
        Ok(direction) => Ok(direction),
        Err(()) => Err(format!(
            "invalid-raw-direction tf={} indicator={} field={} value={:?}",
            timeframe.as_i64(),
            indicator,
            field,
            value
        )),
    }
}

fn take_extracted<T>(value: Option<T>, timeframe: TimeframeSec, field: &str) -> Result<T, String> {
    value.ok_or_else(|| {
        format!(
            "extractor-internal-missing tf={} field={}",
            timeframe.as_i64(),
            field
        )
    })
}

fn build_timeframe(
    history: &IndicatorHistory,
    raw: &RawIndicators,
    timeframe: TimeframeSec,
) -> Result<TimeframeSnapshot, String> {
    let mut errors = Vec::new();
    macro_rules! required {
        ($expression:expr) => {
            match $expression {
                Ok(value) => Some(value),
                Err(error) => {
                    errors.push(error);
                    None
                }
            }
        };
    }

    macro_rules! optional {
        ($expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(error) => {
                    errors.push(error);
                    None
                }
            }
        };
    }

    let open = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Candle,
        IndicatorField::Open,
        0,
    ));
    let high = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Candle,
        IndicatorField::High,
        0,
    ));
    let low = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Candle,
        IndicatorField::Low,
        0,
    ));
    let close = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Candle,
        IndicatorField::Close,
        0,
    ));
    let mean_line = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::MeanLine,
        0,
    ));
    let prev_mean = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::MeanLine,
        1,
    ));
    let down_big = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::DownBig,
        0,
    ));
    let down_outer = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::DownOuter,
        0,
    ));
    let down_small = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::DownSmall,
        0,
    ));
    let down_inner = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::DownInner,
        0,
    ));
    let up_inner = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::UpInner,
        0,
    ));
    let up_small = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::UpSmall,
        0,
    ));
    let up_outer = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::UpOuter,
        0,
    ));
    let up_big = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::UpBig,
        0,
    ));
    let current_cross = required!(read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::CurrentCross,
        0,
    ));
    let zz_dir = required!(read_direction(
        history,
        timeframe,
        IndicatorKey::ZigZagTrend
    ));
    let zz_break_price = required!(read_raw_float(raw, timeframe, "zigzagtrend", "BreakPrice"));
    let volume_gain = required!(read_raw_float(raw, timeframe, "volume_50", "Gain"));
    let volume_gain_min = required!(read_raw_float(raw, timeframe, "volume_50", "GainMin"));
    let volume_gain_max = required!(read_raw_float(raw, timeframe, "volume_50", "GainMax"));
    let buy_sell_gain = required!(read_raw_float(raw, timeframe, "volume_50", "BuySellGain"));

    let smc_direction = required!(read_direction(history, timeframe, IndicatorKey::Smc));
    let smc_direction_swing =
        required!(read_raw_direction(raw, timeframe, "smc", "DirectionSwing"));
    let smc_fp = optional!(read_optional_raw_float(raw, timeframe, "smc", "FP"));
    let smc_high_price = optional!(read_optional_raw_float(raw, timeframe, "smc", "HighPrice"));
    let smc_low_price = optional!(read_optional_raw_float(raw, timeframe, "smc", "LowPrice"));
    let smc_break_price = optional!(read_optional_raw_float(raw, timeframe, "smc", "BreakPrice"));
    let smc_ob_direction = optional!(read_optional_raw_direction(
        raw,
        timeframe,
        "smc",
        "OBDirection"
    ));
    let smc_ob_high = optional!(read_optional_raw_float(raw, timeframe, "smc", "OBHigh"));
    let smc_ob_low = optional!(read_optional_raw_float(raw, timeframe, "smc", "OBLow"));
    let smc_sbp = optional!(read_optional_raw_float(raw, timeframe, "smc", "SBP"));
    let zz_buy_price = optional!(read_optional_raw_float(
        raw,
        timeframe,
        "zigzagtrend",
        "BuyPrice"
    ));
    let zz_sell_price = optional!(read_optional_raw_float(
        raw,
        timeframe,
        "zigzagtrend",
        "SellPrice"
    ));

    if !errors.is_empty() {
        return Err(format!("tf={} {}", timeframe.as_i64(), errors.join(" | ")));
    }

    let open = take_extracted(open, timeframe, "candle.open")?;
    let high = take_extracted(high, timeframe, "candle.high")?;
    let low = take_extracted(low, timeframe, "candle.low")?;
    let close = take_extracted(close, timeframe, "candle.close")?;
    let mean_line = take_extracted(mean_line, timeframe, "mrc_200.mean_line")?;
    let prev_mean = take_extracted(prev_mean, timeframe, "mrc_200.prev_mean_line")?;
    let down_big = take_extracted(down_big, timeframe, "mrc_200.down_big")?;
    let down_outer = take_extracted(down_outer, timeframe, "mrc_200.down_outer")?;
    let down_small = take_extracted(down_small, timeframe, "mrc_200.down_small")?;
    let down_inner = take_extracted(down_inner, timeframe, "mrc_200.down_inner")?;
    let up_inner = take_extracted(up_inner, timeframe, "mrc_200.up_inner")?;
    let up_small = take_extracted(up_small, timeframe, "mrc_200.up_small")?;
    let up_outer = take_extracted(up_outer, timeframe, "mrc_200.up_outer")?;
    let up_big = take_extracted(up_big, timeframe, "mrc_200.up_big")?;
    let current_cross = take_extracted(current_cross, timeframe, "mrc_200.current_cross")?;
    let zz_dir = take_extracted(zz_dir, timeframe, "zigzagtrend.direction")?;
    let zz_break_price = take_extracted(zz_break_price, timeframe, "zigzagtrend.break_price")?;
    let volume_gain = take_extracted(volume_gain, timeframe, "volume_50.gain")?;
    let volume_gain_min = take_extracted(volume_gain_min, timeframe, "volume_50.gain_min")?;
    let volume_gain_max = take_extracted(volume_gain_max, timeframe, "volume_50.gain_max")?;
    let buy_sell_gain = take_extracted(buy_sell_gain, timeframe, "volume_50.buy_sell_gain")?;
    let smc_direction = take_extracted(smc_direction, timeframe, "smc.direction")?;
    let smc_direction_swing =
        take_extracted(smc_direction_swing, timeframe, "smc.direction_swing")?;

    let candle = Candle {
        open,
        high,
        low,
        close,
    };
    let smc = SmcSnapshot {
        direction: smc_direction,
        direction_swing: smc_direction_swing,
        fp: smc_fp,
        high_price: smc_high_price,
        low_price: smc_low_price,
        break_price: smc_break_price,
        ob_direction: smc_ob_direction,
        ob_high: smc_ob_high,
        ob_low: smc_ob_low,
        sbp: smc_sbp,
    };
    let mrc = MrcBands {
        down_big,
        down_outer,
        down_small,
        down_inner,
        mean_line,
        up_inner,
        up_small,
        up_outer,
        up_big,
        current_cross,
        mean_line_slope: mean_line - prev_mean,
    };

    Ok(TimeframeSnapshot {
        candle,
        mrc,
        mrc_signal: mrc::analyze(&mrc, close),
        zz_dir,
        // Zigzag BuyPrice/SellPrice are empty when no local bottom/top exists
        // in the processing window, so preserve that absence as None.
        zz_buy_price,
        zz_sell_price,
        zz_break_price,
        smc,
        volume_gain,
        volume_gain_min,
        volume_gain_max,
        buy_sell_gain,
    })
}

pub fn build_indicator_snapshot(
    history: &IndicatorHistory,
    raw: &RawIndicators,
) -> Result<IndicatorSnapshot, String> {
    let results = [
        ("1m", build_timeframe(history, raw, TF_1M)),
        ("5m", build_timeframe(history, raw, TF_5M)),
        ("15m", build_timeframe(history, raw, TF_15M)),
        ("1h", build_timeframe(history, raw, TF_1H)),
    ];
    let mut errors = Vec::new();
    let mut snapshots = Vec::new();
    for (label, result) in results {
        match result {
            Ok(snapshot) => snapshots.push((label, snapshot)),
            Err(error) => errors.push(error),
        }
    }
    if !errors.is_empty() {
        return Err(format!("indicator-extraction {}", errors.join(" | ")));
    }

    let mut snapshots = snapshots.into_iter();
    let tf1m = snapshots
        .next()
        .map(|(_, snapshot)| snapshot)
        .ok_or_else(|| "indicator-extraction internal=missing-1m-result".to_string())?;
    let tf5m = snapshots
        .next()
        .map(|(_, snapshot)| snapshot)
        .ok_or_else(|| "indicator-extraction internal=missing-5m-result".to_string())?;
    let tf15m = snapshots
        .next()
        .map(|(_, snapshot)| snapshot)
        .ok_or_else(|| "indicator-extraction internal=missing-15m-result".to_string())?;
    let tf1h = snapshots
        .next()
        .map(|(_, snapshot)| snapshot)
        .ok_or_else(|| "indicator-extraction internal=missing-1h-result".to_string())?;

    Ok(IndicatorSnapshot {
        tf1m,
        tf5m,
        tf15m,
        tf1h,
        ..Default::default()
    })
}

pub fn merge_indicator_snapshot(
    base: Option<&IndicatorSnapshot>,
    history: &IndicatorHistory,
    raw: &RawIndicators,
    timeframes: &[i64],
) -> Result<IndicatorSnapshot, String> {
    let Some(base) = base else {
        if timeframes.is_empty() {
            return Err("missing-cached-snapshot no-timeframe-update".to_string());
        }
        return build_indicator_snapshot(history, raw);
    };
    let mut snapshot = *base;
    if timeframes.is_empty() {
        return Ok(snapshot);
    }

    let mut errors = Vec::new();
    for timeframe in timeframes {
        let Some(tf) = timeframe_from_secs(*timeframe) else {
            continue;
        };
        let tf_snapshot = match build_timeframe(history, raw, tf) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        match tf {
            TF_1M => snapshot.tf1m = tf_snapshot,
            TF_5M => snapshot.tf5m = tf_snapshot,
            TF_15M => snapshot.tf15m = tf_snapshot,
            TF_1H => snapshot.tf1h = tf_snapshot,
            _ => {}
        }
    }

    if !errors.is_empty() {
        return Err(format!("indicator-extraction {}", errors.join(" | ")));
    }

    Ok(snapshot)
}

pub fn validate_indicator_snapshot(snapshot: &IndicatorSnapshot) -> Result<(), String> {
    let mut problems = Vec::new();

    for (label, tf) in [
        ("1m", &snapshot.tf1m),
        ("5m", &snapshot.tf5m),
        ("15m", &snapshot.tf15m),
        ("1h", &snapshot.tf1h),
    ] {
        let mut missing = Vec::new();
        if tf.candle.close <= 0.0 {
            missing.push("close");
        }
        if tf.mrc.range() <= 0.0 {
            missing.push("mrc_range");
        }
        if !missing.is_empty() {
            problems.push(format!(
                "{} missing={} zz={} close={:.6} mean={:.6}",
                label,
                missing.join(","),
                tf.zz_dir,
                tf.candle.close,
                tf.mrc.mean_line,
            ));
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("invalid-indicators {}", problems.join(" | ")))
    }
}

pub fn build_sug_info_snapshot(sug_info: &SuggestionInfo) -> SugInfoSnapshot {
    let fast_candle = sug_info.fast_candle.as_ref().map(|fc| FastCandleSnapshot {
        candle: Candle {
            open: fc.enter,
            high: fc.max,
            low: fc.min,
            close: fc.exit,
        },
        active: sug_info.status.is_fast(),
    });
    let sug_candle = Some(Candle {
        open: sug_info.candle.enter,
        high: sug_info.candle.max,
        low: sug_info.candle.min,
        close: sug_info.candle.exit,
    });
    let signal = SignalIndicatorSnapshot {
        ntps: sug_info.indicator.ntps,
    };

    SugInfoSnapshot {
        fast_candle,
        sug_candle,
        signal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lte_strategy_bridge::abi::ModuleIndicatorValue;
    use std::collections::{BTreeMap, HashMap};

    fn mk_history() -> (IndicatorHistory, RawIndicators) {
        let mut history = IndicatorHistory::default();
        let mut snapshot: BTreeMap<
            i64,
            Vec<HashMap<String, HashMap<String, ModuleIndicatorValue>>>,
        > = BTreeMap::new();

        for tf in [60_i64, 300, 900, 3600] {
            let mut candle = HashMap::new();
            candle.insert(
                "candle".to_string(),
                HashMap::from([
                    ("Open".to_string(), ModuleIndicatorValue::Float(100.0)),
                    ("High".to_string(), ModuleIndicatorValue::Float(110.0)),
                    ("Low".to_string(), ModuleIndicatorValue::Float(90.0)),
                    ("Close".to_string(), ModuleIndicatorValue::Float(105.0)),
                ]),
            );
            candle.insert(
                "mrc_200".to_string(),
                HashMap::from([
                    ("DownBig".to_string(), ModuleIndicatorValue::Float(80.0)),
                    ("DownOuter".to_string(), ModuleIndicatorValue::Float(85.0)),
                    ("DownSmall".to_string(), ModuleIndicatorValue::Float(90.0)),
                    ("DownInner".to_string(), ModuleIndicatorValue::Float(95.0)),
                    (
                        "MeanLine".to_string(),
                        ModuleIndicatorValue::Float(100.0 + tf as f64 / 1000.0),
                    ),
                    ("UpInner".to_string(), ModuleIndicatorValue::Float(105.0)),
                    ("UpSmall".to_string(), ModuleIndicatorValue::Float(110.0)),
                    ("UpOuter".to_string(), ModuleIndicatorValue::Float(115.0)),
                    ("UpBig".to_string(), ModuleIndicatorValue::Float(120.0)),
                    ("CurrentCross".to_string(), ModuleIndicatorValue::Float(1.0)),
                ]),
            );
            candle.insert(
                "zigzagtrend".to_string(),
                HashMap::from([
                    (
                        "Direction".to_string(),
                        ModuleIndicatorValue::Direction(Direction::Short),
                    ),
                    ("BuyPrice".to_string(), ModuleIndicatorValue::Float(99.0)),
                    ("SellPrice".to_string(), ModuleIndicatorValue::Float(101.0)),
                    ("BreakPrice".to_string(), ModuleIndicatorValue::Float(100.0)),
                ]),
            );
            candle.insert(
                "smc".to_string(),
                HashMap::from([
                    (
                        "Direction".to_string(),
                        ModuleIndicatorValue::Direction(Direction::Long),
                    ),
                    (
                        "DirectionSwing".to_string(),
                        ModuleIndicatorValue::Direction(Direction::Long),
                    ),
                ]),
            );
            candle.insert(
                "volume_50".to_string(),
                HashMap::from([
                    ("Gain".to_string(), ModuleIndicatorValue::Float(12.5)),
                    ("GainMin".to_string(), ModuleIndicatorValue::Float(1.0)),
                    ("GainMax".to_string(), ModuleIndicatorValue::Float(20.0)),
                    ("BuySellGain".to_string(), ModuleIndicatorValue::Float(-2.5)),
                ]),
            );
            snapshot.insert(tf, vec![candle.clone(), candle]);
        }

        history.init(&snapshot);
        (history, snapshot)
    }

    #[test]
    fn missing_fields_degrade_to_neutral() {
        let history = IndicatorHistory::default();
        let raw = BTreeMap::new();
        match build_indicator_snapshot(&history, &raw) {
            Ok(_) => panic!("expected missing raw indicator error"),
            Err(err) => {
                assert!(err.contains("missing-value"));
                assert!(err.contains("tf=60"));
                assert!(err.contains("tf=300"));
                assert!(err.contains("tf=900"));
                assert!(err.contains("tf=3600"));
            }
        }
    }

    #[test]
    fn typed_both_direction_is_valid() {
        let (mut history, raw) = mk_history();
        let mut init = raw.clone();
        for candles in init.values_mut() {
            for candle in candles {
                candle.insert(
                    "zigzagtrend".to_string(),
                    HashMap::from([
                        (
                            "Direction".to_string(),
                            ModuleIndicatorValue::Direction(Direction::Both),
                        ),
                        ("BreakPrice".to_string(), ModuleIndicatorValue::Float(100.0)),
                    ]),
                );
            }
        }
        history.init(&init);

        let result = build_indicator_snapshot(&history, &raw);
        match result {
            Ok(snapshot) => {
                assert_eq!(snapshot.tf1m.zz_dir, 0);
                assert!(validate_indicator_snapshot(&snapshot).is_ok());
            }
            Err(error) => panic!("typed Both should be valid: {error}"),
        }
    }

    #[test]
    fn string_directions_are_decoded() {
        let (mut history, mut raw) = mk_history();
        for candles in raw.values_mut() {
            for candle in candles {
                candle.insert(
                    "zigzagtrend".to_string(),
                    HashMap::from([
                        (
                            "Direction".to_string(),
                            ModuleIndicatorValue::String("BOTH".to_string()),
                        ),
                        ("BreakPrice".to_string(), ModuleIndicatorValue::Float(100.0)),
                    ]),
                );
                candle.insert(
                    "smc".to_string(),
                    HashMap::from([
                        (
                            "Direction".to_string(),
                            ModuleIndicatorValue::String("LONG".to_string()),
                        ),
                        (
                            "DirectionSwing".to_string(),
                            ModuleIndicatorValue::String("SHORT".to_string()),
                        ),
                        (
                            "OBDirection".to_string(),
                            ModuleIndicatorValue::String("BOTH".to_string()),
                        ),
                    ]),
                );
            }
        }
        history.init(&raw);

        let snapshot =
            build_indicator_snapshot(&history, &raw).expect("string directions should be decoded");
        assert_eq!(snapshot.tf1m.zz_dir, 0);
        assert_eq!(snapshot.tf1m.smc.direction, 1);
        assert_eq!(snapshot.tf1m.smc.direction_swing, -1);
        assert_eq!(snapshot.tf1m.smc.ob_direction, Some(0));
    }

    #[test]
    fn smc_direction_requires_direction_and_accepts_both() {
        let (mut history, raw) = mk_history();
        let mut missing = raw.clone();
        for candles in missing.values_mut() {
            for candle in candles {
                candle
                    .get_mut("smc")
                    .expect("smc test indicator")
                    .remove("Direction");
            }
        }
        history.init(&missing);
        let error = build_indicator_snapshot(&history, &raw)
            .expect_err("missing SMC direction should be reported");
        assert!(error.contains("missing-direction tf=60 indicator=smc"));

        let mut with_both = raw.clone();
        for candles in with_both.values_mut() {
            for candle in candles {
                candle.get_mut("smc").expect("smc test indicator").insert(
                    "Direction".to_string(),
                    ModuleIndicatorValue::Direction(Direction::Both),
                );
            }
        }
        history.init(&with_both);
        let snapshot = build_indicator_snapshot(&history, &raw)
            .expect("typed Both SMC direction should be allowed");
        assert_eq!(snapshot.tf1m.smc.direction, 0);
    }

    #[test]
    fn invalid_smc_direction_is_reported() {
        let (mut history, raw) = mk_history();
        let mut init = raw.clone();
        for candles in init.values_mut() {
            for candle in candles {
                candle.insert(
                    "smc".to_string(),
                    HashMap::from([(
                        "Direction".to_string(),
                        ModuleIndicatorValue::String("not-a-direction".to_string()),
                    )]),
                );
            }
        }
        history.init(&init);

        let error = build_indicator_snapshot(&history, &raw)
            .expect_err("invalid SMC direction should be reported");
        assert!(error.contains("invalid-direction tf=60 indicator=smc"));
        assert!(error.contains("invalid-direction tf=3600 indicator=smc"));
    }

    #[test]
    fn missing_direction_is_reported_as_extraction_error() {
        let (mut history, raw) = mk_history();
        let mut init = raw.clone();
        for candles in init.values_mut() {
            for candle in candles {
                candle
                    .get_mut("zigzagtrend")
                    .expect("zigzag test indicator")
                    .remove("Direction");
            }
        }
        history.init(&init);

        let result = build_indicator_snapshot(&history, &raw);
        match result {
            Ok(_) => panic!("missing direction should fail extraction"),
            Err(error) => {
                assert!(error.contains("missing-direction tf=60 indicator=zigzagtrend"));
                assert!(error.contains("missing-direction tf=3600 indicator=zigzagtrend"));
            }
        }
    }

    #[test]
    fn zigzag_mapping_is_timeframe_local() {
        let (history, raw) = mk_history();
        let snap =
            build_indicator_snapshot(&history, &raw).expect("indicator snapshot should build");
        assert_eq!(snap.tf15m.zz_dir, -1);
        assert_eq!(snap.tf1h.zz_dir, -1);
    }

    #[test]
    fn missing_zigzag_extrema_are_empty_levels() {
        let (history, mut raw) = mk_history();
        for candles in raw.values_mut() {
            for candle in candles {
                if let Some(zigzag) = candle.get_mut("zigzagtrend") {
                    zigzag.remove("BuyPrice");
                    zigzag.remove("SellPrice");
                }
            }
        }

        let snap = build_indicator_snapshot(&history, &raw)
            .expect("missing zigzag extrema should not block snapshot processing");
        assert_eq!(snap.tf1m.zz_buy_price, None);
        assert_eq!(snap.tf1m.zz_sell_price, None);
        assert_eq!(snap.tf1m.zz_break_price, 100.0);
    }

    #[test]
    fn mrc_band_order_and_slope_are_preserved() {
        let (history, raw) = mk_history();
        let snap =
            build_indicator_snapshot(&history, &raw).expect("indicator snapshot should build");
        assert!(snap.tf5m.mrc.ordered());
        assert!(snap.tf5m.mrc.mean_line_slope.abs() < 1e-9);
        assert_eq!(snap.tf5m.mrc.down_big, 80.0);
        assert_eq!(snap.tf5m.mrc.up_big, 120.0);
    }

    #[test]
    fn fast_candle_and_gain_are_read_from_sug_info_and_history() {
        use lte_strategy_bridge::sug_info::{
            AlgoSuggestionTradeStatus, SuggestionInfo, SuggestionInfoCandle,
            SuggestionInfoIndicator,
        };

        let (history, raw) = mk_history();

        let sug_info = SuggestionInfo {
            time: 1,
            system_time: 1,
            symbol: "TEST".to_string(),
            symbol_id: 1,
            symbol_key: "TEST".to_string(),
            status: AlgoSuggestionTradeStatus::FastTrade,
            price: 106.0,
            candle: SuggestionInfoCandle::new(106.0),
            klines: HashMap::new(),
            fast_candle: Some(SuggestionInfoCandle {
                min: 104.0,
                max: 108.0,
                enter: 105.0,
                exit: 107.0,
                qty: 1.0,
                qty_asset: 2.0,
                qtym: 3.0,
                qtym_asset: 4.0,
            }),
            direction: Direction::Long,
            indicator: SuggestionInfoIndicator {
                ntps: 77,
                ntps_fast_time: None,
                trandm: 8.5,
                asset_01: 9.5,
                asset_diff_01: 1.25,
                price_1h: 0.0,
                price_4h: 0.0,
                price_8h: 0.0,
                price_24h: 0.0,
            },
            order_book: None,
            oi: None,
        };

        let sug_snap = build_sug_info_snapshot(&sug_info);
        assert!(sug_snap.fast_candle.is_some());
        assert!(matches!(sug_snap.fast_candle, Some(ref fc) if fc.active));
        assert_eq!(sug_snap.signal.ntps, 77);
        let snap =
            build_indicator_snapshot(&history, &raw).expect("indicator snapshot should build");
        assert_eq!(snap.tf1m.volume_gain, 12.5);
        assert_eq!(snap.tf1m.volume_gain_min, 1.0);
        assert_eq!(snap.tf1m.volume_gain_max, 20.0);
        assert!(snap.tf1m.gain_is_building());
        assert!(!snap.tf1m.gain_is_late());
        assert!(snap.tf1m.gain_progress_pct() > 0.5);
    }

    #[test]
    fn gain_zone_marks_late_when_gain_nears_the_observed_maximum() {
        let mut snap = TimeframeSnapshot {
            volume_gain: 18.0,
            volume_gain_min: 1.0,
            volume_gain_max: 20.0,
            ..Default::default()
        };
        assert_eq!(snap.gain_zone(), crate::model::GainZone::Late);
        assert!(snap.gain_is_late());

        snap.volume_gain = 8.0;
        assert_eq!(snap.gain_zone(), crate::model::GainZone::Building);
        assert!(snap.gain_is_building());
    }
}
