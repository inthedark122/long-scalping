use crate::detection::mrc;
use crate::model::{Candle, IndicatorSnapshot, MrcBands, TimeframeSnapshot};
use lte_strategy_bridge::abi::ModuleIndicatorValue;
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const TF_1M: TimeframeSec = TimeframeSec::Tf1m;
const TF_5M: TimeframeSec = TimeframeSec::Tf5m;

fn timeframe_from_secs(timeframe: i64) -> Option<TimeframeSec> {
    match timeframe {
        60 => Some(TF_1M),
        300 => Some(TF_5M),
        _ => None,
    }
}

fn has_all_timeframes(timeframes: &[i64]) -> bool {
    [60, 300]
        .iter()
        .all(|timeframe| timeframes.contains(timeframe))
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

fn read_optional_float(
    history: &IndicatorHistory,
    timeframe: TimeframeSec,
    indicator: IndicatorKey,
    field: IndicatorField,
    offset: usize,
) -> Result<Option<f64>, String> {
    match history.get_prev(
        IndicatorFieldKey {
            timeframe,
            indicator,
            field,
        },
        offset,
    ) {
        None => Ok(None),
        Some(ModuleIndicatorValue::Float(value)) => Ok(Some(*value)),
        Some(ModuleIndicatorValue::String(value)) => value.parse::<f64>().map(Some).map_err(|_| {
            format!(
                "invalid-optional-float tf={} indicator={} field={} offset={} value={}",
                timeframe.as_i64(),
                indicator.as_name(),
                field.as_name(),
                offset,
                value
            )
        }),
        Some(other) => Err(format!(
            "invalid-optional-value tf={} indicator={} field={} offset={} value={:?}",
            timeframe.as_i64(),
            indicator.as_name(),
            field.as_name(),
            offset,
            other
        )),
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
    // Volume is the only non-MRC input. Missing fields become neutral zeroes
    // so a partial volume feed cannot invalidate the complete MRC snapshot.
    let volume_gain =
        optional!(read_optional_raw_float(raw, timeframe, "volume_50", "Gain")).unwrap_or_default();
    let volume_gain_min = optional!(read_optional_raw_float(
        raw,
        timeframe,
        "volume_50",
        "GainMin"
    ))
    .unwrap_or_default();
    let volume_gain_max = optional!(read_optional_raw_float(
        raw,
        timeframe,
        "volume_50",
        "GainMax"
    ))
    .unwrap_or_default();
    let buy_sell_gain = optional!(read_optional_raw_float(
        raw,
        timeframe,
        "volume_50",
        "BuySellGain"
    ))
    .unwrap_or_default();
    let ntps = optional!(read_optional_float(
        history,
        timeframe,
        IndicatorKey::Ntps,
        IndicatorField::Value,
        0,
    ))
    .unwrap_or_default()
    .round() as i64;

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
    let candle = Candle {
        open,
        high,
        low,
        close,
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
        volume_gain,
        volume_gain_min,
        volume_gain_max,
        buy_sell_gain,
        ntps,
    })
}

pub fn build_indicator_snapshot(
    history: &IndicatorHistory,
    raw: &RawIndicators,
) -> Result<IndicatorSnapshot, String> {
    let results = [
        ("1m", build_timeframe(history, raw, TF_1M)),
        ("5m", build_timeframe(history, raw, TF_5M)),
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
    Ok(IndicatorSnapshot {
        tf1m,
        tf5m,
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
        if !has_all_timeframes(timeframes) {
            return Err(format!(
                "missing-cached-snapshot incremental-indicators tfs=[{}]",
                timeframes
                    .iter()
                    .map(|timeframe| timeframe.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
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

    for (label, tf) in [("1m", &snapshot.tf1m), ("5m", &snapshot.tf5m)] {
        let mut missing = Vec::new();
        if tf.candle.close <= 0.0 {
            missing.push("close");
        }
        if tf.mrc.range() <= 0.0 {
            missing.push("mrc_range");
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
        Err(format!("invalid-indicators {}", problems.join(" | ")))
    }
}
