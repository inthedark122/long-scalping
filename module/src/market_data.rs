use crate::detection::mrc;
use crate::model::{
    Candle, FastCandleSnapshot, IndicatorSnapshot, MrcBands, SignalIndicatorSnapshot,
};
use crate::model::{SugInfoSnapshot, TimeframeSnapshot};
use lte_strategy_bridge::abi::{Direction, ModuleIndicatorValue};
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorHistory, IndicatorKey, TimeframeSec,
};
use lte_strategy_bridge::sug_info::SuggestionInfo;
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
        mrc_signal: mrc::analyze(&mrc, close),
        dp_dir: read_dir(history, timeframe, IndicatorKey::DPSignal),
        zz_dir: read_dir(history, timeframe, IndicatorKey::ZigZagTrend),
        dp_zee: read_raw_float(raw, timeframe, "dpsignal", "Zee"),
        dp_zee1: read_raw_float(raw, timeframe, "dpsignal", "Zee1"),
        zz_buy_price: read_raw_float(raw, timeframe, "zigzagtrend", "BuyPrice"),
        zz_sell_price: read_raw_float(raw, timeframe, "zigzagtrend", "SellPrice"),
        zz_break_price: read_raw_float(raw, timeframe, "zigzagtrend", "BreakPrice"),
        volume_gain: read_float(
            history,
            timeframe,
            IndicatorKey::Volume50,
            IndicatorField::Gain,
            0,
        ),
        volume_gain_min: read_float(
            history,
            timeframe,
            IndicatorKey::Volume50,
            IndicatorField::Min,
            0,
        ),
        volume_gain_max: read_float(
            history,
            timeframe,
            IndicatorKey::Volume50,
            IndicatorField::Max,
            0,
        ),
    }
}

pub fn build_indicator_snapshot(
    history: &IndicatorHistory,
    raw: &RawIndicators,
    live_price: f64,
) -> IndicatorSnapshot {
    IndicatorSnapshot {
        tf1m: build_timeframe(history, raw, TF_1M, Some(live_price)),
        tf5m: build_timeframe(history, raw, TF_5M, None),
        tf15m: build_timeframe(history, raw, TF_15M, None),
        tf1h: build_timeframe(history, raw, TF_1H, None),
        ..Default::default()
    }
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
        if tf.dp_dir == 0 {
            missing.push("dp_dir");
        }
        if tf.zz_dir == 0 {
            missing.push("zz_dir");
        }
        if tf.candle.close <= 0.0 {
            missing.push("close");
        }
        if tf.mrc.range() <= 0.0 {
            missing.push("mrc_range");
        }
        if !missing.is_empty() {
            problems.push(format!(
                "{} missing={} dp={} zz={} close={:.6} zee1={:.6} zee={:.6} mean={:.6}",
                label,
                missing.join(","),
                tf.dp_dir,
                tf.zz_dir,
                tf.candle.close,
                tf.dp_zee1,
                tf.dp_zee,
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
    let sug_direction = if sug_info.direction.is_long() {
        1
    } else if sug_info.direction.is_short() {
        -1
    } else {
        0
    };
    let signal = SignalIndicatorSnapshot {
        ntps: sug_info.indicator.ntps,
    };

    SugInfoSnapshot {
        fast_candle,
        sug_candle,
        sug_direction,
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
            candle.insert(
                "volume_50".to_string(),
                HashMap::from([
                    ("Gain".to_string(), ModuleIndicatorValue::Float(12.5)),
                    ("Min".to_string(), ModuleIndicatorValue::Float(1.0)),
                    ("Max".to_string(), ModuleIndicatorValue::Float(20.0)),
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
        let snap = build_indicator_snapshot(&history, &raw, 101.0);
        assert_eq!(snap.tf1m.dp_dir, 0);
        assert_eq!(snap.tf1m.zz_dir, 0);
        assert_eq!(snap.tf1m.mrc.range(), 0.0);
        assert_eq!(snap.tf1m.candle.close, 101.0);
        assert_eq!(snap.tf1m.volume_gain, 0.0);
        assert_eq!(snap.tf1m.gain_zone(), crate::model::GainZone::Weak);
    }

    #[test]
    fn dp_and_zz_mapping_is_timeframe_local() {
        let (history, raw) = mk_history();
        let snap = build_indicator_snapshot(&history, &raw, 106.0);
        assert_eq!(snap.tf1m.dp_dir, 1);
        assert_eq!(snap.tf5m.dp_dir, 1);
        assert_eq!(snap.tf15m.zz_dir, -1);
        assert_eq!(snap.tf1h.zz_dir, -1);
    }

    #[test]
    fn mrc_band_order_and_slope_are_preserved() {
        let (history, raw) = mk_history();
        let snap = build_indicator_snapshot(&history, &raw, 106.0);
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
        assert!(sug_snap.fast_candle.unwrap().active);
        assert_eq!(sug_snap.signal.ntps, 77);
        let snap = build_indicator_snapshot(&history, &raw, 106.0);
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
