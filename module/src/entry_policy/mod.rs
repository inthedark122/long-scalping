use crate::detection::candle_trend;
use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::model::IndicatorSnapshot;
use crate::pattern_matching;
use crate::state::EngineState;

pub mod notes;
pub mod state;

pub fn analysis_summary(
    state: &EngineState,
    snapshot: &IndicatorSnapshot,
    analysis: Option<&MrcCrossAnalysis>,
    forecast: bool,
    pattern: &pattern_matching::Evaluation,
    price: f64,
    now_ms: u64,
    tick_size: f64,
) -> String {
    let trend = candle_trend::evaluate(
        state.klines.get(crate::market_data::TF_1M),
        now_ms,
        price,
        snapshot.tf1m.mrc.lines(),
        tick_size,
    );
    let mrc = analysis
        .map(|analysis| format!("mrc=[{}]", notes::mrc_analysis(analysis, forecast)))
        .unwrap_or_else(|| "mrc=none".to_string());
    format!("{mrc} {} {}", trend.summary, pattern.summary)
}
