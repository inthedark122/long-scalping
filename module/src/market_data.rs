use crate::model::{Candle, IndicatorSnapshot, MrcBands, TimeframeSnapshot};
use lte_strategy_bridge::abi::{Direction, ModuleIndicatorValue};
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};
use std::collections::{BTreeMap, HashMap};

const TF_1M: TimeframeSec = TimeframeSec::Tf1m;
const TF_5M: TimeframeSec = TimeframeSec::Tf5m;
const TF_15M: TimeframeSec = TimeframeSec::Tf15m;
const TF_1H: TimeframeSec = TimeframeSec::Tf1h;

fn read_float(
    history: &IndicatorHistory,
    timeframe: TimeframeSec,
    indicator: IndicatorKey,
    field: IndicatorField,
    offset: usize,
) -> f64 {
    match history.get_prev(
        IndicatorFieldKey {
            timeframe,
            indicator,
            field,
        },
        offset,
    ) {
        Some(ModuleIndicatorValue::Float(value)) => *value,
        Some(ModuleIndicatorValue::String(value)) => value.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn read_dir(history: &IndicatorHistory, timeframe: TimeframeSec, indicator: IndicatorKey) -> i8 {
    match history.get(IndicatorFieldKey {
        timeframe,
        indicator,
        field: IndicatorField::Direction,
    }) {
        Some(ModuleIndicatorValue::Direction(Direction::Long)) => 1,
        Some(ModuleIndicatorValue::Direction(Direction::Short)) => -1,
        Some(ModuleIndicatorValue::String(value)) if value.eq_ignore_ascii_case("LONG") => 1,
        Some(ModuleIndicatorValue::String(value)) if value.eq_ignore_ascii_case("SHORT") => -1,
        _ => 0,
    }
}

type RawIndicators = BTreeMap<i64, Vec<HashMap<String, HashMap<String, ModuleIndicatorValue>>>>;

fn read_raw_float(
    indicators: &RawIndicators,
    timeframe: TimeframeSec,
    indicator: &str,
    field: &str,
) -> f64 {
    indicators
        .get(&timeframe.as_i64())
        .and_then(|candles| candles.first())
        .and_then(|by_ind| by_ind.get(indicator))
        .and_then(|fields| fields.get(field))
        .and_then(|value| match value {
            ModuleIndicatorValue::Float(v) => Some(*v),
            ModuleIndicatorValue::String(v) => v.parse::<f64>().ok(),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn build_timeframe(
    history: &IndicatorHistory,
    raw: &RawIndicators,
    timeframe: TimeframeSec,
    live_price: Option<f64>,
) -> TimeframeSnapshot {
    let close = live_price.unwrap_or_else(|| {
        read_float(
            history,
            timeframe,
            IndicatorKey::Candle,
            IndicatorField::Close,
            0,
        )
    });
    let candle = Candle {
        open: read_float(
            history,
            timeframe,
            IndicatorKey::Candle,
            IndicatorField::Open,
            0,
        ),
        high: read_float(
            history,
            timeframe,
            IndicatorKey::Candle,
            IndicatorField::High,
            0,
        ),
        low: read_float(
            history,
            timeframe,
            IndicatorKey::Candle,
            IndicatorField::Low,
            0,
        ),
        close,
    };
    let mean_line = read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::MeanLine,
        0,
    );
    let prev_mean = read_float(
        history,
        timeframe,
        IndicatorKey::Mrc200,
        IndicatorField::MeanLine,
        1,
    );
    let mrc = MrcBands {
        down_big: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::DownBig,
            0,
        ),
        down_outer: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::DownOuter,
            0,
        ),
        down_small: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::DownSmall,
            0,
        ),
        down_inner: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::DownInner,
            0,
        ),
        mean_line,
        up_inner: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::UpInner,
            0,
        ),
        up_small: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::UpSmall,
            0,
        ),
        up_outer: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::UpOuter,
            0,
        ),
        up_big: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::UpBig,
            0,
        ),
        current_cross: read_float(
            history,
            timeframe,
            IndicatorKey::Mrc200,
            IndicatorField::CurrentCross,
            0,
        ),
        mean_line_slope: mean_line - prev_mean,
    };

    TimeframeSnapshot {
        candle,
        mrc,
        dp_dir: read_dir(history, timeframe, IndicatorKey::DPSignal),
        zz_dir: read_dir(history, timeframe, IndicatorKey::ZigZagTrend),
        dp_zee: read_raw_float(raw, timeframe, "dpsignal", "Zee"),
        dp_zee1: read_raw_float(raw, timeframe, "dpsignal", "Zee1"),
        zz_buy_price: read_raw_float(raw, timeframe, "zigzagtrend", "BuyPrice"),
        zz_sell_price: read_raw_float(raw, timeframe, "zigzagtrend", "SellPrice"),
        zz_break_price: read_raw_float(raw, timeframe, "zigzagtrend", "BreakPrice"),
    }
}

pub fn build_snapshot(history: &IndicatorHistory, raw: &RawIndicators, live_price: f64) -> IndicatorSnapshot {
    IndicatorSnapshot {
        tf1m: build_timeframe(history, raw, TF_1M, Some(live_price)),
        tf5m: build_timeframe(history, raw, TF_5M, None),
        tf15m: build_timeframe(history, raw, TF_15M, None),
        tf1h: build_timeframe(history, raw, TF_1H, None),
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
                "dpsignal".to_string(),
                HashMap::from([(
                    "Direction".to_string(),
                    ModuleIndicatorValue::Direction(Direction::Long),
                )]),
            );
            candle.insert(
                "zigzagtrend".to_string(),
                HashMap::from([(
                    "Direction".to_string(),
                    ModuleIndicatorValue::Direction(Direction::Short),
                )]),
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
        let snap = build_snapshot(&history, &raw, 101.0);
        assert_eq!(snap.tf1m.dp_dir, 0);
        assert_eq!(snap.tf1m.zz_dir, 0);
        assert_eq!(snap.tf1m.mrc.range(), 0.0);
        assert_eq!(snap.tf1m.candle.close, 101.0);
    }

    #[test]
    fn dp_and_zz_mapping_is_timeframe_local() {
        let (history, raw) = mk_history();
        let snap = build_snapshot(&history, &raw, 106.0);
        assert_eq!(snap.tf1m.dp_dir, 1);
        assert_eq!(snap.tf5m.dp_dir, 1);
        assert_eq!(snap.tf15m.zz_dir, -1);
        assert_eq!(snap.tf1h.zz_dir, -1);
    }

    #[test]
    fn mrc_band_order_and_slope_are_preserved() {
        let (history, raw) = mk_history();
        let snap = build_snapshot(&history, &raw, 106.0);
        assert!(snap.tf5m.mrc.ordered());
        assert!(snap.tf5m.mrc.mean_line_slope.abs() < 1e-9);
        assert_eq!(snap.tf5m.mrc.down_big, 80.0);
        assert_eq!(snap.tf5m.mrc.up_big, 120.0);
    }
}
