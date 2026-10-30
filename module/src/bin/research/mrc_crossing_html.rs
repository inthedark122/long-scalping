use std::error::Error;
use std::fs;
use std::io::{self, BufWriter, Write};

use super::virtual_trade::{
    VirtualPosition, VirtualPredictionResult, VirtualPredictionSummary, VirtualTradeResult,
    VirtualTradeSummary,
};
use serde::Serialize;

use super::data::CandleRow;
use super::mrc_crossing::AnalysisReport;

const HTML_PREDICTION_LIMIT: usize = 500;
const INPUT_SOURCE_DESCRIPTION: &str = "LivTorgEx universal klines; MRC-200 computed locally with ta-rs dev. gain=asset_buy+asset_sell; buy_sell_gain=asset_buy-asset_sell; NTPS unavailable (0).";

#[derive(Debug, Serialize)]
struct PatternDefinitions {
    pattern: &'static str,
    pattern_steps: &'static str,
    stair_sequence: &'static str,
    stair_price_move_pct: &'static str,
    stair_favorable_move_pct: &'static str,
    stair_adverse_move_pct: &'static str,
    staircase_lifecycle: &'static str,
    staircase_transition: &'static str,
    staircase_protected_line: &'static str,
    staircase_extreme_line: &'static str,
    cascade_phase: &'static str,
    cascade_potential_move_pct: &'static str,
    cascade_adverse_move_pct: &'static str,
    cascade_trade_state: &'static str,
    outcome_counts: &'static str,
    resistance: &'static str,
    tp_sl: &'static str,
    trace_outcome: &'static str,
    position_mfe: &'static str,
    win_rate: &'static str,
}

impl Default for PatternDefinitions {
    fn default() -> Self {
        Self {
            pattern: "A stable local movement classification: staircase, break-return, range/trend cycle, or pullback.",
            pattern_steps: "For a staircase, the number of same-direction zigzag legs supporting the classified pattern.",
            stair_sequence: "The MRC line sequence used by the STAIRS_UP or STAIRS_DOWN decision.",
            stair_price_move_pct: "Direction-adjusted net price movement from the beginning of the stair to the current event.",
            stair_favorable_move_pct: "Maximum favorable price excursion from the beginning of the stair.",
            stair_adverse_move_pct: "Maximum adverse price excursion from the beginning of the stair.",
            staircase_lifecycle: "Current staircase state: progressing, pullback, stalled, or invalidated.",
            staircase_transition: "Structural transition that produced the current staircase state.",
            staircase_protected_line: "Protected swing line. Breaking it invalidates the staircase.",
            staircase_extreme_line: "Furthest line reached by the latest confirmed directional leg.",
            cascade_phase: "Current candle-cascade phase: starting, building, mature, counter_move, or reversal.",
            cascade_potential_move_pct: "Continuation range estimated from the active and recent same-direction cascade movements.",
            cascade_adverse_move_pct: "Largest recent opposite-direction cascade movement, used as adverse-excursion risk.",
            cascade_trade_state: "Cascade execution gate explaining whether direction, speed, potential, and adverse risk permit entry.",
            outcome_counts: "Exact rejection, continuation, and neutral outcome counts used by the classifier. They are raw evidence, not trade win rate.",
            resistance: "The opposite reverted MRC line and its barrier state at the signal.",
            tp_sl: "TP and SL are the actual MRC lines used by the executed side, not merely the learned destination.",
            trace_outcome: "Observed movement is profitable when its favorable excursion is larger than its absolute adverse excursion; otherwise it is negative.",
            position_mfe: "For a completed filled position, MFE is the maximum favorable price movement observed before the simulated target, stop, or end-of-history exit. OHLC data cannot determine intrabar order when target and stop share one candle.",
            win_rate: "Wins divided by closed virtual positions for the exact displayed pattern; the position count must be read with the percentage.",
        }
    }
}

#[derive(Debug, Serialize)]
struct HtmlReportData<'a> {
    symbol_key: &'a str,
    input_source: &'static str,
    history_requested_1m: usize,
    history_loaded_1m: usize,
    simulated_entries: usize,
    completed_positions: usize,
    open_positions: usize,
    bypass_entry_blockers: bool,
    bypass_forced_candidates: usize,
    bypass_forced_setups: usize,
    bypass_forced_market_accepted: usize,
    bypass_forced_market_busy: usize,
    direction: Option<&'static str>,
    latest_event_time_ms: Option<i64>,
    trade_summary: &'a VirtualTradeSummary,
    trades: &'a [VirtualTradeResult],
    open_trade: &'a Option<VirtualPosition>,
    prediction_summary: &'a VirtualPredictionSummary,
    prediction_total: usize,
    predictions: &'a [VirtualPredictionResult],
    pending_predictions: usize,
    active_predictions: usize,
    pattern_win_rates: &'a [super::mrc_crossing::PatternWinRateReport],
    stair_statistics: &'a super::mrc_crossing::StairStatistics,
    pattern_performance: &'a [super::mrc_crossing::PatternPerformanceReport],
    parallel_pattern_trades: &'a [super::mrc_crossing::PatternTradeReport],
    active_pattern: Option<&'static str>,
    active_pattern_score: f64,
    pattern_definitions: PatternDefinitions,
    events: &'a [super::mrc_crossing::CrossingEvent],
    forecast: &'a super::mrc_crossing::Forecast,
    candles: &'a [CandleRow],
}

/// Serialize JSON directly into the report file while preserving the script
/// safety guarantee previously provided by `replace('<', "\\u003c")`.
struct HtmlJsonWriter<W> {
    inner: W,
}

impl<W> HtmlJsonWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> Write for HtmlJsonWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut start = 0;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte != b'<' {
                continue;
            }
            self.inner.write_all(&bytes[start..index])?;
            self.inner.write_all(b"\\u003c")?;
            start = index + 1;
        }
        self.inner.write_all(&bytes[start..])?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub(crate) fn write_report(
    path: &str,
    symbol_key: &str,
    history_requested: usize,
    minute_count: usize,
    direction: Option<i8>,
    minute: &[CandleRow],
    report: &AnalysisReport,
) -> Result<(), Box<dyn Error>> {
    let data = HtmlReportData {
        symbol_key,
        input_source: INPUT_SOURCE_DESCRIPTION,
        history_requested_1m: history_requested,
        history_loaded_1m: minute_count,
        simulated_entries: report.trade_summary.trades
            + if report.open_trade.is_some() { 1 } else { 0 },
        completed_positions: report.trade_summary.trades,
        open_positions: if report.open_trade.is_some() { 1 } else { 0 },
        bypass_entry_blockers: report.bypass_entry_blockers,
        bypass_forced_candidates: report.bypass_forced_candidates,
        bypass_forced_setups: report.bypass_forced_setups,
        bypass_forced_market_accepted: report.bypass_forced_market_accepted,
        bypass_forced_market_busy: report.bypass_forced_market_busy,
        direction: direction.map(|value| if value > 0 { "long" } else { "short" }),
        latest_event_time_ms: report.latest_event.as_ref().map(|event| event.time_ms),
        trade_summary: &report.trade_summary,
        trades: &report.trades,
        open_trade: &report.open_trade,
        prediction_summary: &report.prediction_summary,
        prediction_total: report.predictions.len(),
        predictions: &report.predictions[report
            .predictions
            .len()
            .saturating_sub(HTML_PREDICTION_LIMIT)..],
        pending_predictions: report.pending_predictions,
        active_predictions: report.active_predictions,
        pattern_win_rates: &report.pattern_win_rates,
        stair_statistics: &report.stair_statistics,
        pattern_performance: &report.pattern_performance,
        parallel_pattern_trades: &report.parallel_pattern_trades,
        active_pattern: report.active_pattern,
        active_pattern_score: report.active_pattern_score,
        pattern_definitions: PatternDefinitions::default(),
        events: &report.events,
        forecast: &report.forecast,
        candles: minute,
    };
    let (prefix, suffix) = TEMPLATE
        .split_once("__DATA__")
        .ok_or("HTML template is missing the data placeholder")?;
    let file = fs::File::create(path)?;
    let mut output = BufWriter::new(file);
    output.write_all(prefix.as_bytes())?;
    let mut json_writer = HtmlJsonWriter::new(&mut output);
    serde_json::to_writer(&mut json_writer, &data)?;
    json_writer.flush()?;
    output.write_all(suffix.as_bytes())?;
    output.flush()?;
    Ok(())
}

const TEMPLATE: &str = r###"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>MRC Crossing PnL Report</title>
  <style>
    :root {
      color-scheme: dark;
      --bg: #0b1020;
      --panel: #131b2f;
      --panel-2: #19243d;
      --border: #2c3a5a;
      --text: #e8eefc;
      --muted: #91a0bd;
      --green: #55d69b;
      --red: #ff7185;
      --blue: #72a7ff;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, sans-serif;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      line-height: 1.4;
    }
    .page { width: 100%; max-width: 1880px; margin: 0 auto; padding: 28px; }
    h1 { margin: 0 0 5px; font-size: 25px; }
    h2 { margin: 0 0 16px; font-size: 17px; }
    .subtle { color: var(--muted); }
    .header { margin-bottom: 22px; }
    .cards {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 12px;
      margin: 18px 0;
    }
    .card, .panel {
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 12px;
    }
    .card { padding: 14px 16px; }
    .card-label { color: var(--muted); font-size: 12px; }
    .card-value { margin-top: 4px; font-size: 21px; font-weight: 700; }
    .panel { padding: 18px; margin-top: 16px; }
    .chart-wrap { height: 340px; position: relative; }
    .chart-wrap.tall { height: 430px; }
    .stairs-chart-wrap {
      height: 1040px; position: relative; overflow-x: auto; overflow-y: hidden;
      scrollbar-gutter: stable;
    }
    .stairs-chart-scroll-note { margin-top: 8px; color: var(--muted); font-size: 12px; }
    canvas { width: 100%; height: 100%; display: block; }
    .chart-tooltip {
      position: absolute; z-index: 2; display: none; max-width: 420px;
      padding: 10px 12px; background: #0b1020; border: 1px solid var(--border);
      border-radius: 8px; box-shadow: 0 8px 24px rgba(0, 0, 0, .35);
      pointer-events: none; font-size: 12px;
    }
    #stairs-chart-tooltip { position: fixed; }
    #stairs-chart-tooltip { pointer-events: auto; }
    .copy-point { margin-top: 8px; }
    .chart-legend { display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 12px; }
    .legend-item { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); font-size: 12px; }
    .legend-swatch { width: 12px; height: 3px; border-radius: 2px; }
    .stairs-chart-summary {
      display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 12px;
      color: var(--muted); font-size: 12px;
    }
    .stairs-chart-summary strong { color: var(--text); }
    .pattern-filter-bar {
      display: flex; flex-wrap: wrap; align-items: center; gap: 10px;
      margin: 18px 0; padding: 12px 14px; background: var(--panel);
      border: 1px solid var(--border); border-radius: 12px;
    }
    .pattern-filter-label { color: var(--muted); font-size: 12px; font-weight: 700; white-space: nowrap; }
    .pattern-chips { display: flex; flex: 1 1 560px; flex-wrap: wrap; gap: 7px; }
    .pattern-chip, .pattern-filter-action {
      color: var(--muted); background: var(--panel-2); border: 1px solid var(--border);
      border-radius: 999px; padding: 6px 10px; cursor: pointer; font: inherit; font-size: 12px;
    }
    .pattern-chip:hover, .pattern-filter-action:hover { border-color: var(--blue); color: var(--text); }
    .pattern-chip.selected { color: var(--text); border-color: var(--blue); background: rgba(114, 167, 255, .18); }
    .pattern-filter-actions { display: flex; gap: 6px; }
    .pattern-filter-summary { color: var(--muted); font-size: 12px; white-space: nowrap; }
    .pattern-reason-group { margin: 10px 0; }
    .pattern-reason-group summary { cursor: pointer; color: var(--text); font-weight: 700; }
    .pattern-reason-group table { margin-top: 8px; }
    .copy-id, .copy-point, .reason-toggle {
      color: var(--muted); background: var(--panel-2); border: 1px solid var(--border);
      border-radius: 6px; padding: 3px 6px; cursor: pointer; font: inherit; font-size: 11px;
    }
    .copy-id:hover, .copy-point:hover, .reason-toggle:hover { color: var(--text); border-color: var(--blue); }
    .empty {
      position: absolute; inset: 0; display: grid; place-items: center;
      color: var(--muted); pointer-events: none;
    }
    .table-wrap { overflow-x: auto; }
    .table-note { margin: 0 0 10px; font-size: 12px; }
    table { width: 100%; border-collapse: collapse; font-size: 13px; }
    th, td {
      padding: 10px 8px;
      border-bottom: 1px solid var(--border);
      text-align: right;
      white-space: nowrap;
    }
    th { color: var(--muted); font-weight: 600; }
    th:first-child, td:first-child,
    th:nth-child(2), td:nth-child(2),
    th:nth-child(3), td:nth-child(3) { text-align: left; }
    .trade-reason td {
      white-space: normal;
      text-align: left;
      color: var(--muted);
      padding-top: 4px;
      padding-bottom: 12px;
    }
    .trade-reason-label { color: var(--blue); font-weight: 600; }
    .reason-card { margin-top: 8px; }
    .reason-title { color: var(--text); font-weight: 700; margin-bottom: 8px; }
    .reason-section { margin: 8px 0; padding: 8px 10px; background: var(--panel-2); border-radius: 8px; }
    .reason-section-title { color: var(--blue); font-size: 12px; font-weight: 700; text-transform: uppercase; letter-spacing: .04em; margin-bottom: 5px; }
    .reason-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 4px 16px; }
    .reason-item { white-space: normal; }
    .reason-key { color: var(--muted); }
    .reason-value { color: var(--text); }
    .crossing-panel .table-wrap { overflow-x: visible; }
    .crossing-table { width: 100%; min-width: 0; table-layout: fixed; }
    .crossing-table th, .crossing-table td { vertical-align: top; }
    .crossing-table th, .crossing-table td { overflow-wrap: anywhere; }
    .crossing-table td:last-child { word-break: break-word; }
    .crossing-table .numeric { white-space: nowrap; }
    .crossing-table th:nth-child(1), .crossing-table td:nth-child(1) { width: 11%; }
    .crossing-table th:nth-child(2), .crossing-table td:nth-child(2) { width: 9%; }
    .crossing-table th:nth-child(3), .crossing-table td:nth-child(3) { width: 9%; }
    .crossing-table th:nth-child(4), .crossing-table td:nth-child(4) { width: 9%; }
    .crossing-table th:nth-child(5), .crossing-table td:nth-child(5) { width: 23%; }
    .crossing-table th:nth-child(6), .crossing-table td:nth-child(6) { width: 7%; }
    .crossing-table th:nth-child(7), .crossing-table td:nth-child(7) { width: 20%; }
    .crossing-table th:nth-child(8), .crossing-table td:nth-child(8) { width: 12%; }
    .crossing-detail td {
      width: 100%;
      padding: 10px 12px 14px;
      background: var(--panel-2);
      text-align: left;
      white-space: normal;
    }
    .crossing-detail-content { display: grid; gap: 8px; }
    .crossing-detail-title { color: var(--blue); font-weight: 700; }
    .crossing-evidence {
      display: grid; grid-template-columns: max-content minmax(0, 1fr);
      gap: 4px 12px; color: var(--muted); font-size: 12px;
    }
    .crossing-evidence-label { color: var(--text); font-weight: 600; }
    .crossing-evidence code {
      display: block; min-width: 0; overflow-wrap: anywhere; word-break: break-word;
    }
    .line-plan-meta {
      display: flex; flex-wrap: wrap; gap: 6px 14px; margin: 10px 0 8px;
      color: var(--muted); font-size: 12px;
    }
    .line-plan-meta strong { color: var(--text); font-weight: 600; }
    .line-plan-table { width: 100%; min-width: 0; table-layout: fixed; margin-top: 8px; font-size: 12px; background: var(--panel-2); }
    .line-plan-table th, .line-plan-table td { padding: 7px 8px; }
    .line-plan-table th { text-align: left; }
    .line-plan-table th, .line-plan-table td { overflow-wrap: anywhere; }
    .line-plan-table td { text-align: right; white-space: normal; }
    .line-plan-table th:nth-child(1), .line-plan-table td:nth-child(1) { width: 12%; }
    .line-plan-table th:nth-child(2), .line-plan-table td:nth-child(2) { width: 24%; }
    .line-plan-table th:nth-child(3), .line-plan-table td:nth-child(3) { width: 11%; }
    .line-plan-table th:nth-child(4), .line-plan-table td:nth-child(4) { width: 11%; }
    .line-plan-table th:nth-child(5), .line-plan-table td:nth-child(5) { width: 8%; }
    .line-plan-table th:nth-child(6), .line-plan-table td:nth-child(6) { width: 12%; }
    .line-plan-table th:nth-child(7), .line-plan-table td:nth-child(7) { width: 14%; }
    .line-plan-table th:nth-child(8), .line-plan-table td:nth-child(8) { width: 8%; }
    .line-plan-table th:first-child, .line-plan-table td:first-child,
    .line-plan-table th:nth-child(2), .line-plan-table td:nth-child(2),
    .line-plan-table th:nth-child(6), .line-plan-table td:nth-child(6) { text-align: left; }
    .route-role { color: var(--blue); }
    .line-plan-note { margin-top: 8px; color: var(--muted); font-size: 12px; }
    .positive { color: var(--green); }
    .negative { color: var(--red); }
    .prediction-lock-note { display: block; margin-top: 2px; }
    .open { color: var(--blue); }
    code { color: var(--text); white-space: normal; }
    @media (max-width: 700px) {
      .page { padding: 16px; }
      .chart-wrap { height: 260px; }
      .stairs-chart-wrap { height: 860px; }
      .crossing-panel .table-wrap { overflow-x: auto; }
      .crossing-table { table-layout: auto; min-width: 1080px; }
    }
  </style>
</head>
<body>
  <main class="page">
    <section class="header">
      <h1>MRC Crossing PnL Report</h1>
      <div id="context" class="subtle"></div>
      <div id="data-note" class="subtle"></div>
      <div id="mode-note" class="subtle"></div>
    </section>

    <section class="pattern-filter-bar" aria-label="Pattern display selection">
      <span class="pattern-filter-label">Display patterns</span>
      <div id="pattern-filter" class="pattern-chips" role="group" aria-label="Patterns"></div>
      <div class="pattern-filter-actions">
        <button type="button" class="pattern-filter-action" id="select-all-patterns">All</button>
        <button type="button" class="pattern-filter-action" id="clear-patterns">Clear</button>
      </div>
      <div id="pattern-filter-summary" class="pattern-filter-summary"></div>
    </section>

    <section id="cards" class="cards"></section>

    <section class="panel">
      <h2>Cumulative PnL</h2>
      <div class="chart-wrap">
        <canvas id="pnl-chart"></canvas>
        <div id="chart-empty" class="empty">No completed positions</div>
      </div>
    </section>

    <section class="panel">
      <h2>All-pattern PnL comparison</h2>
      <p class="subtle">Each line is a separate parallel diagnostic account that follows one pattern only. This is the counterfactual pattern comparison, not the single selected-position account above. The selection is global across this report.</p>
      <div class="chart-wrap tall" id="patterns-pnl-chart-wrap">
        <canvas id="patterns-pnl-chart"></canvas>
        <div id="patterns-pnl-chart-empty" class="empty">No completed pattern trades</div>
        <div id="patterns-pnl-tooltip" class="chart-tooltip"></div>
      </div>
      <div id="patterns-pnl-legend" class="chart-legend"></div>
    </section>

    <section class="panel">
      <h2>Stairs detection / expectation</h2>
      <p class="subtle">This is the candle-level detector view across the full loaded one-minute history, with all nine MRC lines, staircase evidence, protected/extreme structure, and the expected destination. Colored markers identify detections; the projected band/arrow is the expectation recorded at that candle. Later check/X markers show whether the replay followed or broke that expectation. This is structural correctness, separate from trade PnL.</p>
      <div class="stairs-chart-wrap" id="stairs-chart-wrap">
        <canvas id="stairs-chart"></canvas>
        <div id="stairs-chart-empty" class="empty">No selected staircase detections</div>
        <div id="stairs-chart-tooltip" class="chart-tooltip"></div>
      </div>
      <div class="stairs-chart-scroll-note">The full history is rendered horizontally. Scroll the chart area to inspect earlier candles and staircase expectations.</div>
      <div id="stairs-chart-summary" class="stairs-chart-summary"></div>
      <div id="stairs-chart-legend" class="chart-legend"></div>
    </section>

    <section class="panel">
      <h2>Aggregated stair statistics</h2>
      <p class="subtle">This aggregates every detected zigzag staircase, not only filled positions. Observations count crossing detections; prediction matches include forecast evaluations; selected-position PnL is the actual account route for this run.</p>
      <div id="stair-statistics" class="table-wrap"></div>
    </section>

    <section class="panel">
      <h2>Positions</h2>
      <div id="positions-note" class="subtle table-note"></div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>#</th><th>Side</th><th>Route</th><th>Opened</th>
              <th>Entry</th><th>Target</th><th>Stop</th><th>Closed</th>
              <th>Exit</th><th>Outcome</th><th>Gross PnL</th><th>Fees</th><th>Net PnL</th>
              <th>MFE price move</th><th>MAE price move</th><th>Duration</th>
            </tr>
          </thead>
          <tbody id="positions"></tbody>
        </table>
      </div>
    </section>

    <section class="panel">
      <h2>Prediction outcomes</h2>
      <div id="predictions-note" class="subtle table-note"></div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>#</th><th>Side</th><th>Route</th><th>Created</th>
              <th>Entry</th><th>Target</th><th>Stop</th><th>Resolved</th>
              <th>Filled</th><th>Max/furthest line before return</th><th>Returned to source</th>
              <th>Outcome</th><th>Duration</th>
            </tr>
          </thead>
          <tbody id="predictions"></tbody>
        </table>
      </div>
    </section>

    <section class="panel">
      <h2>Pattern definitions</h2>
      <div class="subtle">
        <p><b>Pattern</b> is a stable local movement classification such as <code>STAIRS_UP</code> or <code>FAST_BREAK_RETURN</code>. The long crossing sequence remains diagnostic only.</p>
        <p><b>Pattern steps</b> and elapsed time describe how much local evidence is currently visible. For stairs, steps are alternating zigzag legs; outcome counts are supporting evidence and the pattern itself selects the direction.</p>
        <p><b>Resistance</b> is the opposite reverted MRC line and its current barrier state. <b>TP / SL</b> are the actual MRC lines used by the executed side.</p>
        <p><b>Max/furthest line before return</b> is the exact MRC line index/name farthest from the source line before the first return; it does not use the changing line price.</p>
        <p><b>Prediction / long result</b> is tied to this crossing's own prediction. It shows whether that prediction reached its target or stop, the target/SL movement, and the actual long position result when a long position was filled. Historical trace evidence is kept separately and is not used as this prediction's result.</p>
        <p><b>Win rate</b> is wins divided by closed virtual positions for the exact displayed pattern. Always read it together with the position count.</p>
      </div>
    </section>

    <section class="panel">
      <h2>Win rate by pattern</h2>
      <div id="patterns" class="table-wrap"></div>
    </section>

    <section class="panel">
      <h2>Adaptive pattern selector</h2>
      <p class="subtle">All recognized patterns are evaluated independently. The active pattern remains selected until a challenger exceeds it by the switch margin.</p>
      <div id="pattern-performance" class="table-wrap"></div>
    </section>

    <section class="panel">
      <h2>Reasons for all pattern trades</h2>
      <p class="subtle">These are the full reasons from the parallel diagnostic accounts. They explain why each pattern selected its direction, target, stop, and barrier. They are not all trades taken by the single selected account.</p>
      <div id="pattern-reasons" class="table-wrap"></div>
    </section>

    <section class="panel crossing-panel">
      <h2>Crossing / price trace</h2>
      <p class="subtle">Long-only execution focus. Each crossing shows its current prediction result, target/SL movement, and filled long-position result when available. Click Details to load the full route plan.</p>
      <div id="crossings" class="table-wrap"></div>
    </section>
  </main>

  <script>
    const DATA = __DATA__;
    // Keep the report focused on the data needed for interactive inspection.
    // The JSON report remains the archival/full-fidelity output.
    const MAX_RENDERED_ROWS = 500;
    const trades = DATA.trades || [];
    const tradeSummary = DATA.trade_summary || {};
    const predictions = DATA.predictions || [];
    const predictionTotal = Number(DATA.prediction_total || predictions.length);
    const summary = DATA.prediction_summary || {};
    const patternWinRates = DATA.pattern_win_rates || [];
    const stairStatistics = DATA.stair_statistics || {};
    const patternPerformance = DATA.pattern_performance || [];
    const parallelPatternTrades = DATA.parallel_pattern_trades || [];
    const activePattern = DATA.active_pattern || null;
    const esc = (value) => String(value ?? "")
      .replaceAll("&", "&amp;").replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;").replaceAll('"', "&quot;");
    const patternFilter = document.getElementById("pattern-filter");
    const patternFilterSummary = document.getElementById("pattern-filter-summary");
    const patternNames = [...new Set([
      ...parallelPatternTrades.map((group) => group.pattern),
      ...patternWinRates.map((pattern) => pattern.pattern),
      ...patternPerformance.map((pattern) => pattern.pattern),
      ...trades.map((trade) => trade.pattern_key),
      ...predictions.map((prediction) => prediction.pattern_key),
      ...(DATA.events || []).map((event) => event.pattern_key),
    ].filter((pattern) => pattern && pattern !== "none"))];
    patternFilter.innerHTML = patternNames.map((pattern) =>
      '<button type="button" class="pattern-chip selected" data-pattern="' + esc(pattern) +
      '" aria-pressed="true">' + esc(pattern) + '</button>'
    ).join("");
    let selectedPatternNames = new Set(patternNames);
    const patternIsSelected = (pattern) => selectedPatternNames.has(pattern);
    const latestRows = (rows) => rows.slice(Math.max(0, rows.length - MAX_RENDERED_ROWS));
    const selectedPredictionTotal = () => patternWinRates
      .filter((pattern) => patternIsSelected(pattern.pattern))
      .reduce((total, pattern) => total + Number(pattern.matches || 0), 0);
    const tableWindowNote = (elementId, total, label, loaded = Math.min(total, MAX_RENDERED_ROWS)) => {
      const omitted = Math.max(0, total - loaded);
      document.getElementById(elementId).textContent = omitted
        ? "Showing " + loaded + " recent " + label + " of " + total + ". Use the JSON report for the complete history."
        : "";
    };
    const updatePatternSelection = () => {
      selectedPatternNames = new Set([...patternFilter.querySelectorAll(".pattern-chip.selected")]
        .map((chip) => chip.dataset.pattern));
      patternFilterSummary.textContent = selectedPatternNames.size + " of " + patternNames.length + " selected";
    };
    updatePatternSelection();
    let selectedTrades = [];
    let selectedPredictions = [];
    let selectedEvents = [];
    let selectedOpenTrade = null;
    const refreshSelectedData = () => {
      selectedTrades = trades.filter((trade) => patternIsSelected(trade.pattern_key));
      selectedPredictions = predictions.filter((prediction) => patternIsSelected(prediction.pattern_key));
      selectedEvents = (DATA.events || []).filter((event) => patternIsSelected(event.pattern_key));
      selectedOpenTrade = DATA.open_trade && patternIsSelected(DATA.open_trade.pattern_key)
        ? DATA.open_trade : null;
    };
    refreshSelectedData();
    const money = (value) => Number(value || 0).toFixed(4) + "%";
    const price = (value) => Number(value || 0).toFixed(6);
    const valueRange = (values) => {
      const finite = values.map(Number).filter((value) => Number.isFinite(value));
      return finite.length ? money(Math.min(...finite)) + " .. " + money(Math.max(...finite)) : "—";
    };
    const date = (value) => value ? new Date(Number(value)).toLocaleString() : "—";
    const sharePart = (value) => String(value ?? "na").replaceAll(":", "_");
    const shareId = (kind, row) => {
      const fields = kind === "event"
        ? [row.time_ms, row.line, row.direction, row.pattern_key || "none"]
        : kind === "trade"
          ? [row.opened_at, row.side, row.source_line, row.target_line, row.pattern_key || "none"]
          : [row.created_at, row.side, row.source_line, row.target_line, row.pattern_key || "none"];
      return [kind, DATA.symbol_key || "unknown", ...fields].map(sharePart).join(":");
    };
    const shareMarkup = (kind, row) => {
      const id = shareId(kind, row);
      return '<button type="button" class="copy-id" title="Copy identifier" aria-label="Copy identifier" data-copy-id="' + esc(id) + '">Copy</button>';
    };
    const reasonMarkup = (kind, index, pattern = "", colspan = 13) =>
      '<button type="button" class="reason-toggle" title="Show full reason" data-reason-kind="' +
      esc(kind) + '" data-reason-index="' + index + '" data-reason-pattern="' + esc(pattern) +
      '" data-reason-colspan="' + colspan + '">Reason</button>';
    const copyToClipboard = (value) => {
      const fallback = () => {
        const area = document.createElement("textarea");
        area.value = value;
        area.setAttribute("readonly", "");
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.appendChild(area);
        area.select();
        document.execCommand("copy");
        area.remove();
      };
      return navigator.clipboard && navigator.clipboard.writeText
        ? navigator.clipboard.writeText(value).catch(fallback)
        : Promise.resolve(fallback());
    };
    document.addEventListener("click", (event) => {
      const crossingButton = event.target.closest(".crossing-toggle");
      if (crossingButton) {
        const row = crossingButton.closest("tr");
        const existing = row && row.nextElementSibling &&
          row.nextElementSibling.classList.contains("crossing-lazy-detail")
          ? row.nextElementSibling : null;
        if (existing) {
          existing.remove();
          crossingButton.textContent = "Details";
          return;
        }
        if (!row) return;
        const details = document.createElement("tr");
        details.className = "crossing-detail crossing-lazy-detail";
        const cell = document.createElement("td");
        cell.colSpan = 8;
        cell.innerHTML = crossingDetailMarkup(
          selectedEvents[Number(crossingButton.dataset.crossingIndex)]);
        details.appendChild(cell);
        row.after(details);
        crossingButton.textContent = "Hide details";
        return;
      }
      const reasonButton = event.target.closest(".reason-toggle");
      if (reasonButton) {
        const index = Number(reasonButton.dataset.reasonIndex);
        let reason = null;
        if (reasonButton.dataset.reasonKind === "position") {
          reason = selectedTrades[index]?.reason;
        } else if (reasonButton.dataset.reasonKind === "prediction") {
          reason = selectedPredictions[index]?.reason;
        } else if (reasonButton.dataset.reasonKind === "parallel") {
          const group = parallelPatternTrades.find((item) =>
            item.pattern === reasonButton.dataset.reasonPattern);
          reason = group?.trades?.[index]?.reason;
        }
        const row = reasonButton.closest("tr");
        const existing = row && row.nextElementSibling &&
          row.nextElementSibling.classList.contains("lazy-reason")
          ? row.nextElementSibling : null;
        if (existing) {
          existing.remove();
          reasonButton.textContent = "Reason";
          return;
        }
        if (!row) return;
        const details = document.createElement("tr");
        details.className = "trade-reason lazy-reason";
        const cell = document.createElement("td");
        cell.colSpan = Number(reasonButton.dataset.reasonColspan || 13);
        cell.innerHTML = '<span class="trade-reason-label">' +
          (reasonButton.dataset.reasonKind === "prediction" ? "Prediction reason:" : "Entry reason:") +
          '</span>' + formatReason(reason || "—");
        details.appendChild(cell);
        row.after(details);
        reasonButton.textContent = "Hide reason";
        return;
      }
      const button = event.target.closest(".copy-id");
      if (!button) return;
      copyToClipboard(button.dataset.copyId || "").then(() => {
        const original = button.textContent;
        button.textContent = "Copied";
        setTimeout(() => { button.textContent = original; }, 1200);
      });
    });
    const shortDate = (value) => value ? new Date(Number(value)).toLocaleString([], {
      month: "short", day: "numeric", hour: "2-digit", minute: "2-digit"
    }) : "—";
    const duration = (opened, closed) => {
      if (!opened || !closed) return "—";
      const minutes = (Number(closed) - Number(opened)) / 60000;
      return minutes.toFixed(2) + "m";
    };
    const humanDuration = (value) => {
      const milliseconds = Number(value);
      if (!Number.isFinite(milliseconds) || milliseconds <= 0) return "—";
      if (milliseconds >= 3600000) {
        const totalMinutes = Math.floor(milliseconds / 60000);
        return Math.floor(totalMinutes / 60) + "h " + String(totalMinutes % 60).padStart(2, "0") + "m";
      }
      if (milliseconds >= 60000) {
        const totalSeconds = Math.floor(milliseconds / 1000);
        return Math.floor(totalSeconds / 60) + "m " + String(totalSeconds % 60).padStart(2, "0") + "s";
      }
      if (milliseconds >= 1000) return (milliseconds / 1000).toFixed(1) + "s";
      return Math.round(milliseconds) + "ms";
    };
    const formatDurationValue = (value) => {
      if (value == null || value === "") return "—";
      const numeric = Number(value);
      return Number.isFinite(numeric) ? humanDuration(numeric) : String(value);
    };
    const lineLabels = ["down_big", "down_outer", "down_small", "down_inner", "mean", "up_inner", "up_small", "up_outer", "up_big"];
    const lineRole = (role) => {
      if (!role || !role.kind) return "unknown";
      if (role.kind === "covered_by_target") {
        return "covered-by-" + (lineLabels[role.target_line] || "line-" + role.target_line);
      }
      return role.kind.replaceAll("_", "-");
    };
    const lineIsCurrent = (plan) => plan.route_role && plan.route_role.kind === "current";
    const lineEta = (plan) => {
      if (plan.time_source === "current") return "now";
      if (plan.time_source === "unavailable") return "unavailable";
      return humanDuration(plan.estimated_time_ms);
    };
    const signedPercent = (value) => {
      const number = Number(value);
      if (!Number.isFinite(number)) return "unavailable";
      return (number >= 0 ? "+" : "") + number.toFixed(3) + "%";
    };
    const outcomeClass = (outcome) => outcome === "profitable" ? "positive" : "negative";
    const predictionSummary = (event) => {
      if (!event.prediction_outcome) return '<span class="subtle">no current prediction</span>';
      const side = Number(event.prediction_side) > 0 ? "LONG" : Number(event.prediction_side) < 0 ? "SHORT" : "—";
      const labels = {
        reached_and_followed: "target reached",
        not_reached_but_followed: "target reached · entry missed",
        prediction_broken: "SL reached",
        unresolved: "unresolved",
      };
      const outcome = labels[event.prediction_outcome] || String(event.prediction_outcome).replaceAll("_", " ");
      const lockedOut = DATA.direction && ((DATA.direction === "long" && Number(event.prediction_side) < 0) ||
        (DATA.direction === "short" && Number(event.prediction_side) > 0));
      const lockText = lockedOut
        ? '<span class="subtle prediction-lock-note">· ' + esc(String(DATA.direction).toUpperCase()) + '-only lock</span>'
        : "";
      const className = event.prediction_outcome === "prediction_broken" ? "negative" : "positive";
      return '<span class="' + className + '">' + esc(side + " " + outcome) + " " +
        esc(signedPercent(event.prediction_result_move_pct)) + '</span>' + lockText;
    };
    const directionalMove = (entry, level, side) => {
      const e = Number(entry);
      const l = Number(level);
      const s = Number(side);
      if (!Number.isFinite(e) || e <= 0 || !Number.isFinite(l) || !Number.isFinite(s) || ![1, -1].includes(s)) return null;
      return (l - e) / e * 100 * s;
    };
    const currentPredictionMove = (event) => {
      const side = Number(event.prediction_side || event.trade_direction || 0);
      const targetMove = directionalMove(event.effective_entry_price, event.target_price, side);
      const stopMove = directionalMove(event.effective_entry_price, event.stop_price, side);
      if (targetMove == null && stopMove == null) return "not available";
      const targetText = targetMove == null ? "target unavailable" : "target " + signedPercent(targetMove);
      const stopText = stopMove == null ? "SL unavailable" : "SL " + signedPercent(stopMove);
      return targetText + " · " + stopText;
    };
    const positionSummary = (event) => {
      if (!event.position_outcome) return '<span class="subtle">no completed long position</span>';
      const outcome = String(event.position_outcome).replaceAll("_", " ");
      const exit = event.position_exit_reason
        ? String(event.position_exit_reason).replaceAll("-", " ") : "exit";
      return '<span class="' + outcomeClass(event.position_outcome) + '">' + esc(outcome) +
        ' ' + esc(signedPercent(event.position_pnl_pct)) + '</span>' +
        ' <span class="subtle">· MFE ' + esc(signedPercent(event.position_mfe_pct)) +
        ' before ' + esc(exit) + '</span>';
    };
    const linePlanDetails = (event) => {
      const plans = Array.isArray(event.line_plans) ? event.line_plans : [];
      if (!plans.length) return "—";
      const rows = plans.map((plan) => {
        const current = lineIsCurrent(plan);
        const lineName = lineLabels[plan.line_idx] || "line-" + plan.line_idx;
        const movement = current ? "entry" : signedPercent(plan.move_pct);
        const movementClass = current ? "" : Number(plan.move_pct) >= 0 ? "positive" : "negative";
        const distance = current ? "entry" : Number(plan.distance_pct).toFixed(3) + "%";
        const stop = plan.stop_loss_set ? Number(plan.stop_loss_pct).toFixed(3) + "%" : "not set";
        return '<tr>' +
          '<th scope="row">' + esc(lineName) + '</th>' +
          '<td class="route-role">' + esc(lineRole(plan.route_role)) + '</td>' +
          '<td class="' + movementClass + '">' + esc(movement) + '</td>' +
          '<td>' + esc(distance) + '</td>' +
          '<td>' + esc(stop) + '</td>' +
          '<td>' + esc(lineEta(plan)) + '</td>' +
          '<td>' + esc(String(plan.time_source || "unknown").replaceAll("_", "-")) + '</td>' +
          '<td>' + Number(plan.historical_samples || 0) + '</td>' +
          '</tr>';
      }).join("");
      const movementTarget = directionalMove(
        event.effective_entry_price,
        event.target_price,
        event.trade_direction,
      );
      const target = event.target_is_movement
        ? "entry movement " + (movementTarget == null ? "unavailable" : signedPercent(movementTarget))
        : event.take_profit_name || event.target_name || "not selected";
      const expectedText = event.expected_price_low != null
        ? price(event.expected_price_low) + " .. " + price(event.expected_price_high)
        : "not available";
      const evidenceText = event.target_line == null
        ? "no prediction"
        : "history=" + event.history_scope +
          " · context=" + event.context_samples +
          " · next-line=" + event.next_line_crosses +
          " · support/resistance=" + event.support_resistance_moves +
          " · price-only=" + event.price_only_moves;
      const ntps = Number(event.ntps);
      const ntpsText = Number.isFinite(ntps) && ntps > 0 ? String(ntps) : "unavailable";
      const possibleMoveBeforeSl = event.position_mfe_pct != null
        ? signedPercent(event.position_mfe_pct) + " before " +
          (event.position_exit_reason === "stop" ? "SL" : event.position_exit_reason === "target" ? "TP" : "exit")
        : "no filled long position";
      return '<div class="crossing-route">' +
        '<div class="crossing-detail-title">' + plans.length +
          ' line plans · target ' + esc(target) + '</div>' +
        '<div class="line-plan-meta">' +
          '<span><strong>Side:</strong> ' + esc(event.trade_direction > 0 ? "LONG" : event.trade_direction < 0 ? "SHORT" : "not selected") + '</span>' +
          '<span><strong>Expected:</strong> ' + esc(expectedText) + '</span>' +
          '<span><strong>Current prediction:</strong> ' + predictionSummary(event) + '</span>' +
          '<span><strong>Prediction movement:</strong> ' + esc(currentPredictionMove(event)) + '</span>' +
          '<span><strong>Long position:</strong> ' + positionSummary(event) + '</span>' +
          '<span><strong>Possible move before SL:</strong> ' + esc(possibleMoveBeforeSl) + '</span>' +
          '<span><strong>Cascade:</strong> ' + esc((event.cascade_phase || "unknown") +
            " · state=" + (event.cascade_trade_state || "not_configured") +
            " · direction=" + (event.cascade_direction > 0 ? "LONG" : event.cascade_direction < 0 ? "SHORT" : "na") +
            " · potential=" + (event.cascade_potential_move_pct == null ? "na" : Number(event.cascade_potential_move_pct).toFixed(3) + "%") +
            " · adverse=" + (event.cascade_adverse_move_pct == null ? "na" : Number(event.cascade_adverse_move_pct).toFixed(3) + "%")) + '</span>' +
          '<span><strong>Historical evidence:</strong> ' + esc(evidenceText) + '</span>' +
        '</div>' +
        '<div class="table-wrap"><table class="line-plan-table"><thead><tr>' +
          '<th>Line</th><th>Route role</th><th>Move</th><th>Distance</th><th>SL</th>' +
          '<th>ETA</th><th>ETA source</th><th>Samples</th>' +
        '</tr></thead><tbody>' + rows + '</tbody></table></div>' +
        '<div class="line-plan-note">Cascade speed: ' +
          esc(event.cascade_speed_pct_per_min == null ? "unavailable" : Number(event.cascade_speed_pct_per_min).toFixed(3) + "%/min") +
          ' · phase: ' + esc(event.cascade_phase || "unknown") +
          ' · potential: ' + esc(event.cascade_potential_move_pct == null ? "unavailable" : Number(event.cascade_potential_move_pct).toFixed(3) + "%") +
          ' · adverse: ' + esc(event.cascade_adverse_move_pct == null ? "unavailable" : Number(event.cascade_adverse_move_pct).toFixed(3) + "%") +
          ' · gate: ' + esc(event.cascade_trade_state || "not_configured") +
          ' · NTPS: ' + esc(ntpsText) + '</div>' +
      '</div>';
    };
    const crossingDetailMarkup = (event) => {
      const evidence = event.target_line == null
        ? "no prediction"
        : "history=" + event.history_scope +
          " context=" + event.context_samples +
          " no_pullback=" + event.no_pullback_count +
          " reverted=" + (event.reverted_line_name || "na") +
          " next=" + event.next_line_crosses +
          " support/resistance=" + event.support_resistance_moves +
          " price-only=" + event.price_only_moves;
      const stair = event.stair_sequence && event.stair_sequence.length
        ? "stair=" + event.stair_sequence.join(">") +
          " net=" + Number(event.stair_price_move_pct || 0).toFixed(3) + "%" +
          " favorable=" + Number(event.stair_favorable_move_pct || 0).toFixed(3) + "%" +
          " adverse=" + Number(event.stair_adverse_move_pct || 0).toFixed(3) + "%" +
          " lifecycle=" + (event.staircase_lifecycle || "na") +
          " transition=" + (event.staircase_transition || "na") +
          " reversal_kind=" + (event.transition || "none") +
          " reversal_state=" + (event.reversal_state || "none") +
          " reversal_reason=" + (event.reversal_reason || "na") +
          " protected=" + (event.staircase_protected_line != null ? lineLabels[event.staircase_protected_line] : "na") +
          " extreme=" + (event.staircase_extreme_line != null ? lineLabels[event.staircase_extreme_line] : "na") +
          " cascade_state=" + (event.cascade_trade_state || "not_configured") +
          " cascade_dir=" + (event.cascade_direction > 0 ? "long" : event.cascade_direction < 0 ? "short" : "na") +
          " cascade_potential=" + (event.cascade_potential_move_pct == null ? "na" : Number(event.cascade_potential_move_pct).toFixed(3) + "%") +
          " cascade_adverse=" + (event.cascade_adverse_move_pct == null ? "na" : Number(event.cascade_adverse_move_pct).toFixed(3) + "%")
        : "";
      return '<div class="crossing-detail-content">' +
        linePlanDetails(event) +
        '<div class="crossing-evidence"><span class="crossing-evidence-label">Historical evidence</span><span>' +
        esc(evidence + (stair ? " " + stair : "")) + '</span></div></div>';
    };
    const signedClass = (value) => Number(value) >= 0 ? "positive" : "negative";
    const percentFrom = (value, reference) => {
      const level = Number(value);
      const base = Number(reference);
      if (!Number.isFinite(level) || !Number.isFinite(base) || base <= 0) return null;
      const percent = (level - base) / base * 100;
      return (percent >= 0 ? "+" : "") + percent.toFixed(3) + "% from entry";
    };

    const parseReason = (reason) => {
      const tokens = String(reason || "").trim().split(/\s+/).filter(Boolean);
      if (!tokens.length || tokens[0] !== "mrc-prediction") return null;
      const values = {};
      tokens.slice(1).forEach((token) => {
        const separator = token.indexOf("=");
        if (separator <= 0) return;
        values[token.slice(0, separator)] = token.slice(separator + 1);
      });
      return values;
    };

    const formatReason = (reason) => {
      const values = parseReason(reason);
      if (!values) return '<code>' + esc(reason || "—") + '</code>';
      const formatLevel = (lineKey, priceKey) => {
        let line = values[lineKey];
        let levelPrice = priceKey ? values[priceKey] : null;
        if (levelPrice == null && line && line.includes("@")) {
          const separator = line.lastIndexOf("@");
          levelPrice = line.slice(separator + 1);
          line = line.slice(0, separator);
        }
        if (line == null && levelPrice == null) return null;
        if (levelPrice == null) return line || "—";
        const percentage = percentFrom(levelPrice, values.entry);
        return (line || "—") + " / " + Number(levelPrice).toFixed(6) +
          (percentage ? " (" + percentage + ")" : "");
      };
      const sections = [
        ["Decision", [
          ["Pattern", "pattern"],
          ["Pattern steps", "pattern_steps"],
          ["Pattern elapsed", "pattern_elapsed_ms", null, "duration"],
          ["Stair sequence", "stair_sequence"],
          ["Stair net price move", "stair_price_move_pct"],
          ["Stair favorable move", "stair_favorable_move_pct"],
          ["Stair adverse move", "stair_adverse_move_pct"],
          ["Stair lifecycle", "stair_lifecycle"],
          ["Stair transition", "stair_transition"],
          ["Protected swing line", "stair_protected_line"],
          ["Latest extreme line", "stair_extreme_line"],
          ["Crossed line", "source_line"],
          ["Learned destination", "learned_line"],
          ["Predicted movement", "predicted_direction"],
          ["Executed trade", "trade_direction"],
        ]],
        ["Execution levels", [
          ["Entry price", "entry"],
          ["Take-profit source", "target_source"],
          ["Take-profit line / price", "tp_line", "tp", "level"],
          ["Stop-loss line / price", "sl_line", "sl", "level"],
        ]],
        ["Volume", [
          ["Volume gain", "volume_gain"],
          ["Volume gain min / max", "volume_gain_min", "volume_gain_max"],
          ["Buy/sell flow", "buy_sell_gain"],
        ]],
        ["Resistance", [
          ["Barrier state / line", "barrier"],
          ["Resistance line / price", "resistance_line", null, "level"],
          ["Reverted line", "reverted_line"],
        ]],
        ["Historical evidence", [
          ["History scope", "history_scope"],
          ["Transition observations", "transition_samples"],
          ["Movement observations", "movement_samples"],
          ["Next-line cases", "next_line_crosses"],
          ["Support/resistance cases", "support_resistance"],
          ["Price-only cases", "price_only_moves"],
          ["Movement wins / losses", "movement_wins", "movement_losses"],
          ["Movement success rate", "movement_success_rate"],
        ]],
        ["Projection", [
          ["Context samples", "context_samples"],
          ["No-pullback count", "no_pullback"],
          ["Market price", "market_price"],
          ["MRC range", "mrc_range"],
          ["Expected price", "expected_price"],
          ["Expected move", "expected_move_pct"],
          ["Favorable / adverse move", "favorable_move_pct", "adverse_move_pct"],
          ["Estimated time", "eta_ms", null, "duration"],
          ["Projection", "projection"],
        ]],
      ];
      const sectionHtml = sections.map(([title, fields]) => {
        const items = fields.map(([label, firstKey, secondKey, valueType]) => {
          if (values[firstKey] == null && (!secondKey || values[secondKey] == null)) return "";
          const value = valueType === "level"
            ? formatLevel(firstKey, secondKey)
            : valueType === "duration"
              ? formatDurationValue(values[firstKey])
            : secondKey && values[secondKey] != null
              ? values[firstKey] + " / " + values[secondKey]
              : values[firstKey];
          return '<div class="reason-item"><span class="reason-key">' + esc(label) +
            ':</span> <span class="reason-value">' + esc(value) + '</span></div>';
        }).join("");
        return items ? '<div class="reason-section"><div class="reason-section-title">' +
          esc(title) + '</div><div class="reason-grid">' + items + '</div></div>' : "";
      }).join("");
      return '<div class="reason-card"><div class="reason-title">MRC prediction</div>' + sectionHtml + '</div>';
    };

    document.getElementById("context").textContent =
      DATA.symbol_key + " · 1m history " + DATA.history_loaded_1m +
      " · direction " + String(DATA.direction || "both").toUpperCase() +
      (DATA.latest_event_time_ms ? " · as of " + date(DATA.latest_event_time_ms) : "");
    document.getElementById("data-note").textContent =
      DATA.input_source || "MRC source unavailable in this report.";
    if (DATA.bypass_entry_blockers) {
      document.getElementById("mode-note").textContent =
        "RESEARCH COUNTERFACTUAL: stair entry blockers bypassed; valid target/stop geometry retained. This is not live execution behavior. " +
        "Forced candidates=" + DATA.bypass_forced_candidates +
        ", setups=" + DATA.bypass_forced_setups +
        ", market accepted=" + DATA.bypass_forced_market_accepted +
        ", market busy=" + DATA.bypass_forced_market_busy + ".";
    }

    const summarizeTrades = (rows) => {
      let net = 0, fees = 0, cumulative = 0, peak = 0, maxDrawdown = 0, durationSum = 0, durationCount = 0;
      const ordered = [...rows].sort((left, right) =>
        Number(left.closed_at || left.opened_at || 0) - Number(right.closed_at || right.opened_at || 0));
      ordered.forEach((trade) => {
        net += Number(trade.pnl_pct || 0);
        fees += Number(trade.entry_fee_pct || 0) + Number(trade.exit_fee_pct || 0);
        cumulative += Number(trade.pnl_pct || 0);
        peak = Math.max(peak, cumulative);
        maxDrawdown = Math.max(maxDrawdown, peak - cumulative);
        if (trade.opened_at && trade.closed_at) {
          durationSum += Number(trade.closed_at) - Number(trade.opened_at);
          durationCount += 1;
        }
      });
      return {
        trades: rows.length,
        winRate: rows.length ? rows.filter((trade) => Number(trade.pnl_pct || 0) > 0).length / rows.length * 100 : 0,
        net,
        fees,
        maxDrawdown,
        minNet: rows.length ? Math.min(...rows.map((trade) => Number(trade.pnl_pct || 0))) : 0,
        maxNet: rows.length ? Math.max(...rows.map((trade) => Number(trade.pnl_pct || 0))) : 0,
        mfeRange: valueRange(rows.map((trade) => trade.mfe_pct)),
        maeRange: valueRange(rows.map((trade) => trade.mae_pct)),
        averageDuration: durationCount ? durationSum / durationCount : 0,
      };
    };
    const summarizePredictions = () => {
      const selected = patternWinRates.filter((pattern) => patternIsSelected(pattern.pattern));
      const total = selected.reduce((count, pattern) => count + Number(pattern.matches || 0), 0);
      const filled = selected.reduce((count, pattern) => count + Number(pattern.filled_predictions || 0), 0);
      const followed = selected.reduce((count, pattern) => count + Number(pattern.followed_predictions || 0), 0);
      return {
        predictions: total,
        filled,
        followedRate: total ? followed / total * 100 : 0,
      };
    };
    const renderCards = () => {
      const tradeStats = summarizeTrades(selectedTrades);
      const predictionStats = summarizePredictions();
      const cardData = [
        ["Positions", tradeStats.trades],
        ["Win rate", tradeStats.winRate.toFixed(1) + "%"],
        ["Net PnL", tradeStats.net.toFixed(4) + "%"],
        ["Min net PnL", tradeStats.minNet.toFixed(4) + "%"],
        ["Max net PnL", tradeStats.maxNet.toFixed(4) + "%"],
        ["MFE price move min..max", tradeStats.mfeRange],
        ["MAE price move min..max", tradeStats.maeRange],
        ["Fees", tradeStats.fees.toFixed(4) + "%"],
        ["Max drawdown", tradeStats.maxDrawdown.toFixed(4) + "%"],
        ["Avg duration", tradeStats.averageDuration
          ? (tradeStats.averageDuration / 60000).toFixed(2) + "m" : "—"],
        ["Predictions", predictionStats.predictions],
        ["Filled predictions", predictionStats.filled],
        ["Follow rate", predictionStats.followedRate.toFixed(1) + "%"],
      ];
      document.getElementById("cards").innerHTML = cardData.map(([label, value]) =>
        '<div class="card"><div class="card-label">' + esc(label) +
        '</div><div class="card-value">' + esc(value) + '</div></div>'
      ).join("");
    };

    const renderCrossings = () => {
      const visibleEvents = selectedEvents.slice(-500).reverse();
      const crossingRows = visibleEvents.map((event, index) => {
      const sourceIndex = selectedEvents.length - 1 - index;
      const direction = Number(event.direction || 0) > 0 ? "UP" : "DOWN";
      const learned = event.target_name
        ? (event.pattern_kind || "pattern") + " steps=" + (event.pattern_steps || 0) + " -> " +
          event.target_name + " @ " + price(event.predicted_line_price) +
          " (" + (event.predicted_direction > 0 ? "up" : "down") + ")"
        : "—";
      const exits = event.take_profit_name || event.stop_loss_name
        ? "TP " + (event.take_profit_name || "—") + " @ " + price(event.target_price) +
          "; SL " + (event.stop_loss_name || "—") + " @ " + price(event.stop_price)
        : "—";
      const resistance = event.barrier_state + ":" + (event.barrier_line != null
        ? (event.barrier_line_name || event.barrier_line) + " @ " + price(event.barrier_price)
        : "na");
      const compactResult = predictionSummary(event) +
        (event.position_outcome ? '<br><span class="subtle">LONG ' + positionSummary(event) + '</span>' : '');
      return '<tr><td class="numeric">' + esc(date(event.time_ms)) + ' ' + shareMarkup("event", event) + ' ' +
        '<button type="button" class="reason-toggle crossing-toggle" title="Show route details" data-crossing-index="' +
        sourceIndex + '">Details</button></td><td>' + direction +
        ' ' + esc(event.line_name) + '</td><td class="numeric">' + price(event.market_price) +
        '</td><td class="numeric">' + price(event.effective_entry_price) +
        '</td><td>' + esc(learned) + '</td><td>' + esc(event.trade_direction > 0 ? "LONG" : event.trade_direction < 0 ? "SHORT" : "—") +
        '</td><td>' + esc(exits) + '<br><span class="subtle">' + esc(resistance) + '</span></td>' +
        '<td>' + compactResult + '</td></tr>';
      }).join("");
      document.getElementById("crossings").innerHTML = selectedEvents.length
        ? '<table class="crossing-table"><thead><tr><th>Time</th><th>Cross</th><th>Market price</th><th>Entry</th>' +
          '<th>Prediction</th><th>Side</th><th>TP / SL</th><th>Prediction / long result</th></tr></thead><tbody>' + crossingRows + '</tbody></table>'
        : '<div class="subtle">No crossing observations for selected patterns</div>';
    };

    const renderStairStatistics = () => {
      const byPattern = stairStatistics.by_pattern || [];
      const bySteps = stairStatistics.by_steps || [];
      const duration = (value) => Number(value || 0) / 60000;
      const patternRows = byPattern.map((pattern) =>
        '<tr><td><code>' + esc(pattern.pattern) + '</code></td><td>' + pattern.observations + '</td><td>' +
        Number(pattern.average_steps || 0).toFixed(2) + '</td><td>' + duration(pattern.average_elapsed_ms).toFixed(1) +
        'm</td><td>' + Number(pattern.average_volume_gain || 0).toFixed(2) + '</td><td>' +
        pattern.prediction_matches + '</td><td>' + pattern.filled_predictions + '</td><td>' +
        pattern.followed_predictions + ' / ' + pattern.broken_predictions + '</td><td>' +
        pattern.closed_positions + '</td><td>' + pattern.wins + ' / ' + pattern.losses + '</td><td>' +
        Number(pattern.win_rate_pct || 0).toFixed(1) + '%</td><td>' + money(pattern.total_gross_pnl_pct) +
        '</td><td>' + money(pattern.total_fees_pct) + '</td><td class="' + signedClass(pattern.total_pnl_pct) + '">' +
        money(pattern.total_pnl_pct) + '</td></tr>'
      ).join('');
      const stepRows = bySteps.map((step) =>
        '<tr><td><code>' + esc(step.pattern) + '</code></td><td>' + step.steps + '</td><td>' +
        step.observations + '</td><td>' + duration(step.average_elapsed_ms).toFixed(1) + 'm</td><td>' +
        Number(step.average_volume_gain || 0).toFixed(2) + '</td></tr>'
      ).join('');
      const total = '<div class="subtle">All stairs: <b>' + (stairStatistics.observations || 0) +
        '</b> observations · <b>' + (stairStatistics.prediction_matches || 0) + '</b> prediction matches · <b>' +
        (stairStatistics.followed_predictions || 0) + '</b> followed / <b>' + (stairStatistics.broken_predictions || 0) +
        '</b> broken · selected trades <b>' + (stairStatistics.closed_positions || 0) + '</b> · net <span class="' +
        signedClass(stairStatistics.total_pnl_pct) + '">' + money(stairStatistics.total_pnl_pct) + '</span></div>';
      document.getElementById('stair-statistics').innerHTML = total +
        (patternRows ? '<h3>By direction</h3><table><thead><tr><th>Pattern</th><th>Observations</th><th>Avg steps</th><th>Avg elapsed</th><th>Avg volume</th><th>Predictions</th><th>Filled</th><th>Followed / broken</th><th>Closed</th><th>Wins / losses</th><th>Win rate</th><th>Gross</th><th>Fees</th><th>Net</th></tr></thead><tbody>' + patternRows + '</tbody></table>' : '') +
        (stepRows ? '<h3>By zigzag depth</h3><table><thead><tr><th>Pattern</th><th>Steps</th><th>Observations</th><th>Avg elapsed</th><th>Avg volume</th></tr></thead><tbody>' + stepRows + '</tbody></table>' : '');
    };
    renderStairStatistics();

    const renderPatternTables = () => {
      const visibleWinRates = patternWinRates.filter((pattern) => patternIsSelected(pattern.pattern));
      const parallelByPattern = new Map(parallelPatternTrades.map((group) => [group.pattern, group]));
      const patternRows = visibleWinRates.map((pattern) =>
        '<tr><td><code>' + esc(pattern.pattern) + '</code></td><td>' + pattern.matches + '</td><td>' +
        pattern.filled_predictions + '</td><td>' + pattern.closed_positions + '</td><td>' +
        pattern.wins + ' / ' + pattern.losses + '</td><td>' + Number(pattern.win_rate_pct || 0).toFixed(1) + '%</td><td>' +
        pattern.followed_predictions + ' / ' + pattern.broken_predictions + '</td><td>' +
        money(pattern.total_gross_pnl_pct) + '</td><td>' + money(pattern.total_fees_pct) +
        '</td><td class="' + signedClass(pattern.total_pnl_pct) + '">' + money(pattern.total_pnl_pct) +
        '</td><td>' + money(pattern.average_win_pnl_pct) + '</td><td>' +
        money(pattern.average_loss_pnl_pct) + '</td><td>' +
        valueRange((parallelByPattern.get(pattern.pattern)?.trades || []).map((trade) => trade.mfe_pct)) +
        '</td><td>' + valueRange((parallelByPattern.get(pattern.pattern)?.trades || [])
          .map((trade) => trade.mae_pct)) + '</td></tr>'
      ).join("");
      document.getElementById("patterns").innerHTML = visibleWinRates.length
        ? '<table><thead><tr><th>Pattern</th><th>Matches</th><th>Filled</th><th>Closed</th>' +
          '<th>Wins / losses</th><th>Win rate</th><th>Followed / broken</th><th>Gross</th><th>Fees</th><th>Net</th>' +
          '<th>Average win</th><th>Average loss</th><th>MFE price min..max</th><th>MAE price min..max</th></tr></thead><tbody>' +
          patternRows + '</tbody></table>'
        : '<div class="subtle">No selected patterns with completed positions</div>';

      const visiblePerformance = patternPerformance.filter((pattern) => patternIsSelected(pattern.pattern));
      const performanceRows = visiblePerformance.map((pattern) =>
        '<tr class="' + (pattern.pattern === activePattern ? 'active-row' : '') + '"><td><code>' + esc(pattern.pattern) + '</code></td><td>' +
        pattern.observations + '</td><td>' + pattern.trades + '</td><td>' + Number(pattern.win_rate_pct || 0).toFixed(1) + '%</td><td>' +
        money(pattern.total_pnl_pct) + '</td><td>' + money(pattern.average_pnl_pct) + '</td><td>' + money(pattern.ema_pnl_pct) +
        '</td><td>' + money(pattern.last_pnl_pct) + '</td><td>' + (pattern.pattern === activePattern ? 'ACTIVE' : '') + '</td></tr>'
      ).join('');
      document.getElementById("pattern-performance").innerHTML = performanceRows
        ? '<table><thead><tr><th>Pattern</th><th>Observations</th><th>Trades</th><th>Win rate</th><th>Total net</th><th>Average net</th><th>EMA net</th><th>Last net</th><th>State</th></tr></thead><tbody>' + performanceRows + '</tbody></table>'
        : '<div class="subtle">No selected pattern performance</div>';
    };

    const patternColors = ["#72a7ff", "#55d69b", "#ff7185", "#d7a6ff", "#ffbf69"];
    const buildParallelSeries = () => parallelPatternTrades
      .filter((group) => patternIsSelected(group.pattern))
      .map((group, groupIndex) => {
      const completed = [...(group.trades || [])].sort((left, right) =>
        Number(left.closed_at || left.opened_at || 0) - Number(right.closed_at || right.opened_at || 0));
      let cumulative = 0;
      const points = [];
      if (completed.length) {
        points.push({
          time: Number(completed[0].opened_at || completed[0].closed_at || 0),
          value: 0,
          trade: null,
          tradeIndex: 0,
        });
        completed.forEach((trade, tradeIndex) => {
          cumulative += Number(trade.pnl_pct || 0);
          points.push({
            time: Number(trade.closed_at || trade.opened_at || 0),
            value: cumulative,
            trade,
            tradeIndex: tradeIndex + 1,
          });
        });
      }
      return {
        pattern: group.pattern || "UNKNOWN",
        color: patternColors[groupIndex % patternColors.length],
        points,
        trades: completed,
        summary: group.summary || {},
      };
      }).filter((series) => series.points.length);
    let parallelSeries = buildParallelSeries();

    let allPatternChartState = null;
    const allPatternChart = document.getElementById("patterns-pnl-chart");
    const allPatternTooltip = document.getElementById("patterns-pnl-tooltip");

    const compactReason = (reason) => {
      const values = parseReason(reason);
      if (!values) return esc(reason || "—");
      const fields = [
        ["pattern", "pattern"], ["source_line", "source"], ["learned_line", "learned"],
        ["trade_direction", "trade"], ["barrier", "barrier"],
        ["volume_gain", "volume"], ["buy_sell_gain", "buy/sell"],
        ["transition_samples", "transitions"], ["movement_samples", "movements"],
        ["expected_move_pct", "expected move"],
      ];
      return fields.filter(([key]) => values[key] != null)
        .map(([key, label]) => label + "=" + values[key]).join(" · ");
    };

    const renderPatternReasons = () => {
      const groups = parallelPatternTrades.filter((group) => patternIsSelected(group.pattern)).map((group) => {
        const groupTrades = group.trades || [];
        const summary = group.summary || {};
        const visibleTrades = latestRows(groupTrades);
        const rows = visibleTrades.map((trade, index) => {
          const sourceIndex = groupTrades.length - visibleTrades.length + index;
          const side = Number(trade.side) > 0 ? "LONG" : "SHORT";
          const fees = Number(trade.entry_fee_pct || 0) + Number(trade.exit_fee_pct || 0);
          const pnlClass = signedClass(trade.pnl_pct);
          const route = side + " " + trade.source_name + " → " + trade.target_name;
          return '<tr><td>' + (sourceIndex + 1) + ' ' + shareMarkup("trade", trade) + ' ' +
            reasonMarkup("parallel", sourceIndex, group.pattern, 13) + '</td><td>' + esc(date(trade.closed_at)) +
            '</td><td>' + esc(side) + '</td><td>' + esc(route) + '</td><td>' +
            price(trade.entry_price) + '</td><td>' + price(trade.target_price) + '</td><td>' +
            price(trade.stop_price) + '</td><td>' + esc(trade.exit_reason) + '</td><td>' +
            money(trade.gross_pnl_pct) + '</td><td class="negative">-' + money(fees) +
            '</td><td class="' + pnlClass + '">' + money(trade.pnl_pct) + '</td><td>' +
            money(trade.mfe_pct) + '</td><td>' + money(trade.mae_pct) + '</td></tr>';
        }).join("");
        const net = money(summary.total_pnl_pct);
        const mfeRange = valueRange(groupTrades.map((trade) => trade.mfe_pct));
        const maeRange = valueRange(groupTrades.map((trade) => trade.mae_pct));
        const windowNote = groupTrades.length > MAX_RENDERED_ROWS
          ? '<div class="subtle table-note">Showing the latest ' + MAX_RENDERED_ROWS +
            ' trades of ' + groupTrades.length + '. Full results remain in the embedded report data.</div>'
          : '';
        return '<details class="pattern-reason-group" open><summary><code>' +
          esc(group.pattern) + '</code> · ' + groupTrades.length + ' closed · net ' +
          '<span class="' + signedClass(summary.total_pnl_pct) + '">' + net +
          '</span> · MFE ' + mfeRange + ' · MAE ' + maeRange + '</summary>' + windowNote + (rows
            ? '<table><thead><tr><th>#</th><th>Closed</th><th>Side</th><th>Route</th><th>Entry</th>' +
              '<th>Target</th><th>Stop</th><th>Exit</th><th>Gross</th><th>Fees</th><th>Net</th><th>MFE price</th><th>MAE price</th></tr></thead><tbody>' +
              rows + '</tbody></table>'
            : '<div class="subtle">No completed trades for this pattern.</div>') +
          '</details>';
      }).join("");
      document.getElementById("pattern-reasons").innerHTML = groups ||
        '<div class="subtle">No parallel pattern trades</div>';
    };
    renderPatternReasons();

    const drawAllPatternChart = () => {
      const empty = document.getElementById("patterns-pnl-chart-empty");
      empty.style.display = parallelSeries.length ? "none" : "grid";
      allPatternChartState = null;
      if (!parallelSeries.length) {
        allPatternChart.getContext("2d").clearRect(0, 0, allPatternChart.width, allPatternChart.height);
        return;
      }
      const rect = allPatternChart.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      allPatternChart.width = Math.max(1, Math.floor(rect.width * ratio));
      allPatternChart.height = Math.max(1, Math.floor(rect.height * ratio));
      const ctx = allPatternChart.getContext("2d");
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
      const width = rect.width, height = rect.height;
      const pad = { left: 58, right: 18, top: 22, bottom: 38 };
      const points = parallelSeries.flatMap((series) => series.points);
      const values = points.map((point) => point.value);
      let min = Math.min(...values), max = Math.max(...values);
      if (min === max) { min -= 1; max += 1; }
      const span = max - min;
      min -= span * 0.08; max += span * 0.08;
      const times = points.map((point) => point.time).filter((time) => Number.isFinite(time));
      const minTime = Math.min(...times), maxTime = Math.max(...times);
      const useTime = Number.isFinite(minTime) && Number.isFinite(maxTime) && maxTime > minTime;
      const maxPointCount = Math.max(...parallelSeries.map((series) => series.points.length));
      const xFor = (point, index) => useTime
        ? pad.left + (point.time - minTime) / (maxTime - minTime) * (width - pad.left - pad.right)
        : pad.left + index / Math.max(1, maxPointCount - 1) * (width - pad.left - pad.right);
      const y = (value) => pad.top + (max - value) / (max - min) * (height - pad.top - pad.bottom);
      ctx.clearRect(0, 0, width, height);
      ctx.font = "12px system-ui";
      ctx.strokeStyle = "#2c3a5a";
      ctx.fillStyle = "#91a0bd";
      ctx.lineWidth = 1;
      for (let index = 0; index <= 4; index++) {
        const value = min + (max - min) * index / 4;
        const lineY = y(value);
        ctx.beginPath(); ctx.moveTo(pad.left, lineY); ctx.lineTo(width - pad.right, lineY); ctx.stroke();
        ctx.fillText(value.toFixed(2) + "%", 4, lineY + 4);
      }
      ctx.strokeStyle = "#66789e";
      ctx.setLineDash([5, 4]);
      ctx.beginPath(); ctx.moveTo(pad.left, y(0)); ctx.lineTo(width - pad.right, y(0)); ctx.stroke();
      ctx.setLineDash([]);
      const chartPoints = [];
      parallelSeries.forEach((series) => {
        ctx.strokeStyle = series.color;
        ctx.lineWidth = 2;
        ctx.beginPath();
        const seriesPoints = series.points.map((point, index) => {
          const chartPoint = { series, point, x: xFor(point, index), y: y(point.value) };
          chartPoints.push(chartPoint);
          return chartPoint;
        });
        seriesPoints.forEach((point, index) => {
          if (index === 0) ctx.moveTo(point.x, point.y);
          else ctx.lineTo(point.x, point.y);
        });
        ctx.stroke();
        ctx.fillStyle = series.color;
        seriesPoints.slice(1).forEach((point) => {
          ctx.beginPath(); ctx.arc(point.x, point.y, 3, 0, Math.PI * 2); ctx.fill();
        });
      });
      ctx.fillStyle = "#91a0bd";
      ctx.textAlign = "center";
      const labelTimes = [...new Set([minTime, (minTime + maxTime) / 2, maxTime])];
      labelTimes.forEach((time) => {
        const point = { time };
        const labelX = useTime ? xFor(point, 0) : pad.left;
        ctx.fillText(shortDate(time), labelX, height - 10);
      });
      ctx.textAlign = "left";
      allPatternChartState = { chartPoints, rect };
    };

    const hidePatternTooltip = () => { allPatternTooltip.style.display = "none"; };
    allPatternChart.addEventListener("mousemove", (event) => {
      if (!allPatternChartState) return;
      const rect = allPatternChart.getBoundingClientRect();
      const mouseX = event.clientX - rect.left;
      const mouseY = event.clientY - rect.top;
      const candidates = allPatternChartState.chartPoints.filter((point) => point.point.trade);
      const nearest = candidates.reduce((best, point) => {
        const distance = Math.hypot(point.x - mouseX, point.y - mouseY);
        return !best || distance < best.distance ? { point, distance } : best;
      }, null);
      if (!nearest || nearest.distance > 14) { hidePatternTooltip(); return; }
      const point = nearest.point;
      const trade = point.point.trade;
      const side = Number(trade.side) > 0 ? "LONG" : "SHORT";
      const route = side + " " + trade.source_name + " → " + trade.target_name;
      allPatternTooltip.innerHTML = '<b>' + esc(point.series.pattern) + '</b> · trade #' +
        point.point.tradeIndex + '<br>Closed: ' + esc(date(trade.closed_at)) +
        '<br>Route: ' + esc(route) + '<br>Trade net: <span class="' + signedClass(trade.pnl_pct) + '">' +
        money(trade.pnl_pct) + '</span> · cumulative: <span class="' + signedClass(point.point.value) + '">' +
        money(point.point.value) + '</span><br>MFE price move: ' + money(trade.mfe_pct) +
        ' · MAE price move: ' + money(trade.mae_pct) + '<br>Exit: ' + esc(trade.exit_reason) +
        '<br><span class="reason-key">' + compactReason(trade.reason) + '</span>';
      allPatternTooltip.style.display = "block";
      const tooltipWidth = allPatternTooltip.offsetWidth;
      const tooltipHeight = allPatternTooltip.offsetHeight;
      allPatternTooltip.style.left = Math.max(8, Math.min(mouseX + 14, rect.width - tooltipWidth - 8)) + "px";
      allPatternTooltip.style.top = Math.max(8, Math.min(mouseY + 14, rect.height - tooltipHeight - 8)) + "px";
    });
    allPatternChart.addEventListener("mouseleave", hidePatternTooltip);

    const stairsChart = document.getElementById("stairs-chart");
    const stairsChartWrap = document.getElementById("stairs-chart-wrap");
    const stairsChartEmpty = document.getElementById("stairs-chart-empty");
    const stairsChartTooltip = document.getElementById("stairs-chart-tooltip");
    const stairsChartSummary = document.getElementById("stairs-chart-summary");
    const stairsChartLegend = document.getElementById("stairs-chart-legend");
    const candles = DATA.candles || [];
    const stairsColors = { STAIRS_UP: "#72a7ff", STAIRS_DOWN: "#ffbf69" };
    const mrcLineKeys = ["down_big", "down_outer", "down_small", "down_inner", "mean_line", "up_inner", "up_small", "up_outer", "up_big"];
    const mrcLineColors = ["#536386", "#60739b", "#6d82aa", "#7d91b8", "#e8eefc", "#8b9fc5", "#7d91b8", "#6d82aa", "#60739b"];
    let stairsChartState = null;
    let stairsTooltipEvent = null;

    const isStairsPattern = (pattern) => pattern === "STAIRS_UP" || pattern === "STAIRS_DOWN";
    const stairsColor = (event) => stairsColors[event.pattern_key] || "#d7a6ff";
    const stairsLineName = (line) => {
      if (line == null || line === "") return "unavailable";
      const numeric = Number(line);
      return Number.isInteger(numeric) && lineLabels[numeric]
        ? lineLabels[numeric] + " (#" + numeric + ")" : "unavailable";
    };
    const stairsSequence = (event) => (Array.isArray(event.stair_sequence) && event.stair_sequence.length
      ? event.stair_sequence.map(stairsLineName).join(" → ") : "unavailable");
    const activeStairSummary = (event) => {
      if (!event.active_stair_pattern) return "none";
      const direction = Number(event.active_stair_direction) > 0 ? "LONG" :
        Number(event.active_stair_direction) < 0 ? "SHORT" : "—";
      const route = event.active_stair_route_available ? "route available" :
        "waiting for same-direction route";
      const expected = event.active_stair_expected_line == null
        ? "expectation unavailable"
        : "next " + stairsLineName(event.active_stair_expected_line);
      return event.active_stair_pattern + " · " + direction + " · " + expected + " · " + route;
    };
    const stairsOutcome = (event) => {
      switch (event.prediction_outcome) {
        case "reached_and_followed":
          return { label: "followed (correct)", className: "positive", marker: "followed", price: event.target_price };
        case "not_reached_but_followed":
          return { label: "direction followed · entry missed", className: "positive", marker: "followed", price: event.target_price };
        case "prediction_broken":
          return { label: "broken (wrong)", className: "negative", marker: "broken", price: event.stop_price };
        case "unresolved":
          return { label: "unresolved", className: "", marker: "unresolved", price: event.market_price };
        default:
          return {
            label: event.target_line == null ? "not evaluated" : "expectation only · not queued",
            className: "",
            marker: "not-queued",
            price: event.market_price,
          };
      }
    };
    const stairDirection = (event) => Number(event.predicted_direction || event.trade_direction || 0) > 0
      ? "LONG" : Number(event.predicted_direction || event.trade_direction || 0) < 0 ? "SHORT" : "—";
    const finitePositive = (value) => Number.isFinite(Number(value)) && Number(value) > 0;
    const hasPositionPnl = (event) => event.position_pnl_pct != null && event.position_pnl_pct !== "" &&
      Number.isFinite(Number(event.position_pnl_pct));
    const stairTradeIsSelected = (trade) => isStairsPattern(trade.pattern_key) &&
      patternIsSelected(trade.pattern_key);
    const positionTradeForEvent = (event) => trades.find((trade) =>
      stairTradeIsSelected(trade) &&
      Number(trade.signal_time_ms) === Number(event.time_ms) &&
      Number(trade.source_line) === Number(event.line) &&
      trade.pattern_key === event.pattern_key);
    const eventCandle = (event) => {
      let best = null;
      let distance = Infinity;
      candles.forEach((candle) => {
        const candidateDistance = Math.abs(Number(candle.time_ms) - Number(event.time_ms));
        if (Number.isFinite(candidateDistance) && candidateDistance < distance) {
          best = candle;
          distance = candidateDistance;
        }
      });
      return best;
    };

    const drawArrow = (ctx, fromX, fromY, toX, toY, color) => {
      if (![fromX, fromY, toX, toY].every(Number.isFinite)) return;
      const angle = Math.atan2(toY - fromY, toX - fromX);
      const head = 7;
      ctx.save();
      ctx.strokeStyle = color;
      ctx.fillStyle = color;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([5, 4]);
      ctx.beginPath(); ctx.moveTo(fromX, fromY); ctx.lineTo(toX, toY); ctx.stroke();
      ctx.setLineDash([]);
      ctx.beginPath();
      ctx.moveTo(toX, toY);
      ctx.lineTo(toX - head * Math.cos(angle - Math.PI / 6), toY - head * Math.sin(angle - Math.PI / 6));
      ctx.lineTo(toX - head * Math.cos(angle + Math.PI / 6), toY - head * Math.sin(angle + Math.PI / 6));
      ctx.closePath(); ctx.fill();
      ctx.restore();
    };

    const drawOutcomeMarker = (ctx, point) => {
      const outcome = stairsOutcome(point.event);
      if (!point.event.prediction_outcome || outcome.marker === "unresolved") return;
      if (![point.outcomeX, point.outcomeY].every(Number.isFinite)) return;
      ctx.save();
      ctx.strokeStyle = outcome.marker === "broken" ? "#ff7185" : "#55d69b";
      ctx.fillStyle = "#0b1020";
      ctx.lineWidth = 2;
      if (outcome.marker === "broken") {
        ctx.beginPath(); ctx.moveTo(point.outcomeX - 6, point.outcomeY - 6); ctx.lineTo(point.outcomeX + 6, point.outcomeY + 6); ctx.stroke();
        ctx.beginPath(); ctx.moveTo(point.outcomeX + 6, point.outcomeY - 6); ctx.lineTo(point.outcomeX - 6, point.outcomeY + 6); ctx.stroke();
      } else {
        ctx.beginPath(); ctx.arc(point.outcomeX, point.outcomeY, 6, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
        ctx.beginPath(); ctx.moveTo(point.outcomeX - 3, point.outcomeY); ctx.lineTo(point.outcomeX - 1, point.outcomeY + 3); ctx.lineTo(point.outcomeX + 4, point.outcomeY - 3); ctx.stroke();
      }
      ctx.restore();
    };

    const renderStairsChartSummary = (allStairs, visibleStairs) => {
      const followed = allStairs.filter((event) =>
        event.prediction_outcome === "reached_and_followed" ||
        event.prediction_outcome === "not_reached_but_followed").length;
      const broken = allStairs.filter((event) => event.prediction_outcome === "prediction_broken").length;
      const unresolved = allStairs.filter((event) => event.prediction_outcome === "unresolved").length;
      const notQueued = allStairs.filter((event) => !event.prediction_outcome && event.target_line != null).length;
      const notEvaluated = allStairs.filter((event) => !event.prediction_outcome && event.target_line == null).length;
      const expected = allStairs.filter((event) => event.target_line != null).length;
      const up = allStairs.filter((event) => event.pattern_key === "STAIRS_UP").length;
      const down = allStairs.filter((event) => event.pattern_key === "STAIRS_DOWN").length;
      const omitted = allStairs.length - visibleStairs.length;
      const evaluated = followed + broken;
      const followedRate = evaluated ? (followed / evaluated * 100).toFixed(1) + "%" : "—";
      const completedStairPositions = trades.filter(stairTradeIsSelected);
      const stairPnl = completedStairPositions.reduce((total, trade) =>
        total + Number(trade.pnl_pct), 0);
      const stairPnlClass = stairPnl >= 0 ? "positive" : "negative";
      const stairPnlText = completedStairPositions.length
        ? signedPercent(stairPnl) + " · " + completedStairPositions.length + " closed"
        : "—";
      const latest = allStairs[allStairs.length - 1];
      const latestOutcome = latest && stairsOutcome(latest);
      const latestText = latest
        ? "<span><strong>Latest:</strong> " + esc(latest.pattern_key) + " at " +
          esc(stairsLineName(latest.line)) + " → expected " +
          esc(latest.target_line == null ? "unavailable" : stairsLineName(latest.target_line)) +
          " · " + esc(latest.staircase_lifecycle || "lifecycle unavailable") +
          " · <span class=\"" + (latestOutcome.className || "subtle") + "\">" +
          esc(latestOutcome.label) + "</span></span>" : "";
      stairsChartSummary.innerHTML = [
        "<span><strong>Detections:</strong> " + allStairs.length + "</span>",
        "<span><strong>Expected destination:</strong> " + expected + "</span>",
        "<span class=\"positive\"><strong>Followed:</strong> " + followed + "</span>",
        "<span class=\"negative\"><strong>Broken:</strong> " + broken + "</span>",
        "<span><strong>Followed / evaluated:</strong> " + followedRate + "</span>",
        "<span><strong>Queued but unresolved:</strong> " + unresolved + "</span>",
        "<span><strong>Expectation only / not queued:</strong> " + notQueued + "</span>",
        "<span><strong>No expectation:</strong> " + notEvaluated + "</span>",
        "<span><strong>Up / down:</strong> " + up + " / " + down + "</span>",
        "<span class=\"" + stairPnlClass + "\"><strong>Selected stair PnL:</strong> " + stairPnlText + "</span>",
        latestText,
        omitted > 0 ? "<span>Markers show the latest " + visibleStairs.length + " staircase detections.</span>" : "",
      ].join("");
    };

    const drawStairsChart = () => {
      const allSelectedStairs = selectedEvents.filter((event) => isStairsPattern(event.pattern_key));
      const visibleCandles = candles;
      const visibleStart = Number(visibleCandles[0]?.time_ms);
      const visibleEnd = Number(visibleCandles[visibleCandles.length - 1]?.time_ms);
      const visibleStairs = allSelectedStairs.filter((event) => {
        const time = Number(event.time_ms);
        return !Number.isFinite(visibleStart) || !Number.isFinite(visibleEnd) || (time >= visibleStart && time <= visibleEnd);
      });
      stairsChartEmpty.textContent = !candles.length
        ? "No 1m candle data in this report"
        : allSelectedStairs.length ? "No staircase detections in the loaded candle history" : "No selected staircase detections";
      stairsChartEmpty.style.display = visibleCandles.length ? "none" : "grid";
      renderStairsChartSummary(allSelectedStairs, visibleStairs);
      stairsChartLegend.innerHTML =
        '<span class="legend-item"><span class="legend-swatch" style="background:#55d69b"></span>bull candle</span>' +
        '<span class="legend-item"><span class="legend-swatch" style="background:#ff7185"></span>bear candle</span>' +
        '<span class="legend-item"><span class="legend-swatch" style="background:#e8eefc"></span>MRC mean</span>' +
        '<span class="legend-item"><span class="legend-swatch" style="background:#72a7ff"></span>STAIRS_UP</span>' +
        '<span class="legend-item"><span class="legend-swatch" style="background:#ffbf69"></span>STAIRS_DOWN</span>' +
        '<span class="legend-item">outer ring = active stair primary</span>' +
        '<span class="legend-item">dashed arrow/band = expectation</span>' +
        '<span class="legend-item">protected/extreme ticks = stair structure</span>' +
        '<span class="legend-item"><span class="legend-swatch" style="background:#d7a6ff"></span>selected stair PnL</span>' +
        '<span class="legend-item positive">✓ followed</span>' +
        '<span class="legend-item negative">× broken</span>';
      stairsChartState = null;
      if (!visibleCandles.length) {
        stairsChart.getContext("2d").clearRect(0, 0, stairsChart.width, stairsChart.height);
        return;
      }
      const ratio = window.devicePixelRatio || 1;
      const viewportWidth = stairsChartWrap.clientWidth;
      const desiredWidth = Math.max(viewportWidth, visibleCandles.length * 2.4 + 96);
      const maxCanvasWidth = Math.max(viewportWidth, Math.floor(32767 / ratio));
      stairsChart.style.width = Math.min(desiredWidth, maxCanvasWidth) + "px";
      const rect = stairsChart.getBoundingClientRect();
      stairsChart.width = Math.max(1, Math.floor(rect.width * ratio));
      stairsChart.height = Math.max(1, Math.floor(rect.height * ratio));
      const ctx = stairsChart.getContext("2d");
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
      const width = rect.width, height = rect.height;
      const pad = { left: 76, right: 20, top: 20, bottom: 34 };
      const pnlHeight = Math.max(64, Math.min(110, height * 0.13));
      const pnlTop = height - pad.bottom - pnlHeight;
      const volumeHeight = Math.max(48, Math.min(92, height * 0.18));
      const volumeBottom = pnlTop - 18;
      const volumeTop = volumeBottom - volumeHeight;
      const priceBottom = volumeTop - 20;
      const priceHeight = Math.max(1, priceBottom - pad.top);
      const plotWidth = Math.max(1, width - pad.left - pad.right);
      const times = visibleCandles.map((candle) => Number(candle.time_ms)).filter(Number.isFinite);
      const minTime = times.length ? times.reduce((minimum, time) => Math.min(minimum, time), Infinity) : 0;
      const maxTime = times.length ? times.reduce((maximum, time) => Math.max(maximum, time), -Infinity) : 0;
      const useTime = maxTime > minTime;
      const xForIndex = (index) => pad.left + index / Math.max(1, visibleCandles.length - 1) * plotWidth;
      const xForTime = (time) => {
        const numeric = Number(time);
        if (!Number.isFinite(numeric)) return null;
        if (!useTime) return pad.left + plotWidth / 2;
        return pad.left + Math.max(0, Math.min(1, (numeric - minTime) / (maxTime - minTime))) * plotWidth;
      };
      const rawPrices = [];
      visibleCandles.forEach((candle) => {
        [candle.open, candle.high, candle.low, candle.close].forEach((value) => {
          if (finitePositive(value)) rawPrices.push(Number(value));
        });
        mrcLineKeys.forEach((key) => {
          if (finitePositive(candle.mrc?.[key])) rawPrices.push(Number(candle.mrc[key]));
        });
      });
      visibleStairs.forEach((event) => {
        [event.market_price, event.predicted_line_price, event.target_price, event.stop_price,
          event.expected_price_low, event.expected_price_high].forEach((value) => {
          if (finitePositive(value)) rawPrices.push(Number(value));
        });
      });
      if (!rawPrices.length) {
        ctx.clearRect(0, 0, width, height);
        return;
      }
      let minPrice = rawPrices.reduce((minimum, value) => Math.min(minimum, value), Infinity);
      let maxPrice = rawPrices.reduce((maximum, value) => Math.max(maximum, value), -Infinity);
      const span = Math.max(maxPrice - minPrice, minPrice * 0.001, 1e-9);
      minPrice -= span * 0.06;
      maxPrice += span * 0.06;
      const yForPrice = (value) => {
        const numeric = Number(value);
        return Number.isFinite(numeric) ? pad.top + (maxPrice - numeric) / (maxPrice - minPrice) * priceHeight : null;
      };
      const candleWidth = Math.max(2, Math.min(12, plotWidth / Math.max(1, visibleCandles.length) * 0.62));
      ctx.clearRect(0, 0, width, height);
      ctx.font = "12px system-ui";
      ctx.lineWidth = 1;
      for (let index = 0; index <= 4; index++) {
        const value = minPrice + (maxPrice - minPrice) * index / 4;
        const lineY = yForPrice(value);
        ctx.strokeStyle = "#2c3a5a";
        ctx.beginPath(); ctx.moveTo(pad.left, lineY); ctx.lineTo(width - pad.right, lineY); ctx.stroke();
        ctx.fillStyle = "#91a0bd";
        ctx.textAlign = "right";
        ctx.fillText(price(value), pad.left - 8, lineY + 4);
      }
      ctx.strokeStyle = "#2c3a5a";
      ctx.beginPath(); ctx.moveTo(pad.left, volumeTop); ctx.lineTo(width - pad.right, volumeTop); ctx.stroke();
      ctx.fillStyle = "#91a0bd";
      ctx.textAlign = "left";
      ctx.fillText("volume / buy-sell", pad.left, volumeTop + 14);

      mrcLineKeys.forEach((key, lineIndex) => {
        const points = visibleCandles.map((candle, index) => ({
          x: useTime ? xForTime(candle.time_ms) : xForIndex(index),
          y: yForPrice(candle.mrc?.[key]),
        })).filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));
        if (!points.length) return;
        ctx.save();
        ctx.strokeStyle = mrcLineColors[lineIndex];
        ctx.globalAlpha = lineIndex === 4 ? 0.95 : 0.55;
        ctx.lineWidth = lineIndex === 4 ? 2 : 1;
        ctx.beginPath();
        points.forEach((point, index) => {
          if (index === 0) ctx.moveTo(point.x, point.y); else ctx.lineTo(point.x, point.y);
        });
        ctx.stroke();
        ctx.restore();
      });

      const gainValues = visibleCandles.map((candle) => Number(candle.gain)).filter(Number.isFinite);
      const flowValues = visibleCandles.map((candle) => Number(candle.buy_sell_gain)).filter(Number.isFinite);
      const volumeMax = Math.max(
        1,
        gainValues.reduce((maximum, value) => Math.max(maximum, Math.abs(value)), 0),
        flowValues.reduce((maximum, value) => Math.max(maximum, Math.abs(value)), 0),
      );
      const volumeZero = volumeTop + (volumeBottom - volumeTop) / 2;
      const volumeScale = (volumeBottom - volumeTop) / 2 * 0.82 / volumeMax;
      visibleCandles.forEach((candle, index) => {
        const x = useTime ? xForTime(candle.time_ms) : xForIndex(index);
        const gain = Number(candle.gain);
        const flow = Number(candle.buy_sell_gain);
        [[gain, "#72a7ff", -candleWidth * 0.45], [flow, "#ffbf69", candleWidth * 0.05]].forEach(([value, color, offset]) => {
          if (!Number.isFinite(value)) return;
          const barHeight = Math.abs(value) * volumeScale;
          ctx.fillStyle = color;
          ctx.globalAlpha = 0.72;
          ctx.fillRect(x + offset, value >= 0 ? volumeZero - barHeight : volumeZero, candleWidth * 0.4, barHeight);
          ctx.globalAlpha = 1;
        });
      });
      ctx.strokeStyle = "#66789e";
      ctx.setLineDash([4, 4]);
      ctx.beginPath(); ctx.moveTo(pad.left, volumeZero); ctx.lineTo(width - pad.right, volumeZero); ctx.stroke();
      ctx.setLineDash([]);

      visibleCandles.forEach((candle, index) => {
        const x = useTime ? xForTime(candle.time_ms) : xForIndex(index);
        const openY = yForPrice(candle.open), closeY = yForPrice(candle.close);
        const highY = yForPrice(candle.high), lowY = yForPrice(candle.low);
        if (![x, openY, closeY, highY, lowY].every(Number.isFinite)) return;
        const color = Number(candle.close) >= Number(candle.open) ? "#55d69b" : "#ff7185";
        ctx.strokeStyle = color;
        ctx.fillStyle = color;
        ctx.lineWidth = 1;
        ctx.beginPath(); ctx.moveTo(x, highY); ctx.lineTo(x, lowY); ctx.stroke();
        ctx.fillRect(x - candleWidth / 2, Math.min(openY, closeY), candleWidth,
          Math.max(1, Math.abs(closeY - openY)));
      });

      const visibleReportEvents = visibleStairs.filter((event) => {
        const time = Number(event.time_ms);
        return Number.isFinite(time) && (!useTime || time >= minTime && time <= maxTime);
      });
      const pnlTrades = trades
        .filter(stairTradeIsSelected)
        .filter((trade) => {
          const time = Number(trade.signal_time_ms);
          return Number.isFinite(time) && (!useTime || time >= minTime && time <= maxTime);
        })
        .sort((left, right) => Number(left.signal_time_ms) - Number(right.signal_time_ms));
      let cumulativeStairPnl = 0;
      const pnlPoints = [{ time: minTime, value: 0, event: null }];
      pnlTrades.forEach((trade) => {
        cumulativeStairPnl += Number(trade.pnl_pct);
        pnlPoints.push({
          time: Number(trade.signal_time_ms),
          value: cumulativeStairPnl,
          trade,
        });
      });
      const pnlValues = pnlPoints.map((point) => point.value);
      const pnlMin = Math.min(0, ...pnlValues);
      const pnlMax = Math.max(0, ...pnlValues);
      const pnlSpan = Math.max(pnlMax - pnlMin, 0.01);
      const pnlPlotTop = pnlTop + 22;
      const pnlPlotBottom = height - pad.bottom - 4;
      const yForPnl = (value) => pnlPlotTop + (pnlMax - Number(value)) / pnlSpan * (pnlPlotBottom - pnlPlotTop);
      ctx.strokeStyle = "#2c3a5a";
      ctx.beginPath(); ctx.moveTo(pad.left, pnlTop); ctx.lineTo(width - pad.right, pnlTop); ctx.stroke();
      ctx.fillStyle = "#91a0bd";
      ctx.textAlign = "left";
      ctx.fillText("selected stair PnL", pad.left, pnlTop + 14);
      ctx.textAlign = "right";
      ctx.fillStyle = cumulativeStairPnl >= 0 ? "#55d69b" : "#ff7185";
      ctx.fillText(
        pnlTrades.length ? signedPercent(cumulativeStairPnl) : "no completed stair positions",
        width - pad.right,
        pnlTop + 14,
      );
      const pnlZeroY = yForPnl(0);
      ctx.strokeStyle = "#66789e";
      ctx.setLineDash([4, 4]);
      ctx.beginPath(); ctx.moveTo(pad.left, pnlZeroY); ctx.lineTo(width - pad.right, pnlZeroY); ctx.stroke();
      ctx.setLineDash([]);
      if (pnlTrades.length) {
        ctx.save();
        ctx.strokeStyle = cumulativeStairPnl >= 0 ? "#55d69b" : "#ff7185";
        ctx.lineWidth = 1.8;
        ctx.beginPath();
        pnlPoints.forEach((point, index) => {
          const x = xForTime(point.time);
          const y = yForPnl(point.value);
          if (index === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
        });
        ctx.stroke();
        pnlPoints.slice(1).forEach((point) => {
          const x = xForTime(point.time);
          const y = yForPnl(point.value);
          if (![x, y].every(Number.isFinite)) return;
          ctx.fillStyle = Number(point.trade.pnl_pct) >= 0 ? "#55d69b" : "#ff7185";
          ctx.beginPath(); ctx.arc(x, y, 2.5, 0, Math.PI * 2); ctx.fill();
        });
        ctx.restore();
      }
      const lastByPattern = {};
      visibleReportEvents.forEach((event, index) => { lastByPattern[event.pattern_key] = index; });
      const chartPoints = [];
      const mrcLinePrice = (candle, line) => {
        if (line == null || line === "") return null;
        const numeric = Number(line);
        const key = Number.isInteger(numeric) ? mrcLineKeys[numeric] : null;
        return key && candle ? candle.mrc?.[key] : null;
      };
      const drawStructureTick = (ctx, x, y, color, label, direction) => {
        if (![x, y].every(Number.isFinite)) return;
        ctx.save();
        ctx.strokeStyle = color;
        ctx.fillStyle = color;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(x - 9, y);
        ctx.lineTo(x + 9, y);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.font = "10px system-ui";
        ctx.textAlign = direction > 0 ? "left" : "right";
        ctx.fillText(label, x + (direction > 0 ? 12 : -12), y + 3);
        ctx.restore();
      };
      visibleReportEvents.forEach((event, eventIndex) => {
        const x = xForTime(event.time_ms);
        const candle = eventCandle(event);
        const detectionPrice = finitePositive(event.market_price) ? event.market_price : candle?.close;
        const detectionY = yForPrice(detectionPrice);
        if (![x, detectionY].every(Number.isFinite)) return;
        const color = stairsColor(event);
        const structureDirection = stairDirection(event) === "SHORT" ? -1 : 1;
        drawStructureTick(
          ctx,
          x,
          yForPrice(mrcLinePrice(candle, event.staircase_protected_line)),
          color,
          "protected",
          structureDirection,
        );
        drawStructureTick(
          ctx,
          x,
          yForPrice(mrcLinePrice(candle, event.staircase_extreme_line)),
          color,
          "extreme",
          structureDirection,
        );
        const targetPrice = finitePositive(event.predicted_line_price)
          ? Number(event.predicted_line_price) : Number(event.target_price);
        const targetY = yForPrice(targetPrice);
        const activeOverride = event.active_stair_pattern && event.active_stair_pattern !== event.pattern_key;
        const activeTargetPrice = finitePositive(event.active_stair_expected_price)
          ? Number(event.active_stair_expected_price)
          : mrcLinePrice(candle, event.active_stair_expected_line);
        const activeTargetY = yForPrice(activeTargetPrice);
        const latest = lastByPattern[event.pattern_key] === eventIndex;
        const expectedLow = yForPrice(event.expected_price_low);
        const expectedHigh = yForPrice(event.expected_price_high);
        const outcome = stairsOutcome(event);
        const outcomeX = event.prediction_resolved_at != null ? xForTime(event.prediction_resolved_at) : null;
        const outcomeY = yForPrice(outcome.price);
        const step = plotWidth / Math.max(1, visibleCandles.length - 1);
        const projectionEnd = Number.isFinite(outcomeX)
          ? Math.max(x + 18, outcomeX) : Math.min(width - pad.right, x + step * 12);
        if (Number.isFinite(expectedLow) && Number.isFinite(expectedHigh)) {
          ctx.save();
          ctx.fillStyle = color;
          ctx.globalAlpha = latest ? 0.12 : 0.05;
          ctx.fillRect(x, Math.min(expectedLow, expectedHigh), Math.max(2, projectionEnd - x), Math.abs(expectedHigh - expectedLow));
          ctx.restore();
        }
        if (Number.isFinite(targetY)) drawArrow(ctx, x, detectionY, projectionEnd, targetY, color);
        if (activeOverride && Number.isFinite(activeTargetY)) {
          drawArrow(
            ctx,
            x,
            detectionY,
            projectionEnd,
            activeTargetY,
            stairsColors[event.active_stair_pattern] || "#d7a6ff",
          );
        }
        const point = { event, x, y: detectionY, targetX: projectionEnd, targetY, outcomeX, outcomeY };
        chartPoints.push(point);
        drawOutcomeMarker(ctx, point);
        ctx.save();
        if (event.active_stair_pattern && event.active_stair_pattern !== event.pattern_key) {
          ctx.strokeStyle = stairsColors[event.active_stair_pattern] || "#d7a6ff";
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.arc(x, detectionY, latest ? 10 : 8, 0, Math.PI * 2);
          ctx.stroke();
        }
        ctx.fillStyle = color;
        ctx.strokeStyle = "#0b1020";
        ctx.lineWidth = latest ? 2 : 1;
        ctx.beginPath();
        ctx.arc(x, detectionY, latest ? 6 : 4.5, 0, Math.PI * 2);
        ctx.fill(); ctx.stroke();
        ctx.restore();
      });
      if (useTime) {
        const latestCandleX = xForTime(visibleCandles[visibleCandles.length - 1].time_ms);
        ctx.strokeStyle = "#91a0bd";
        ctx.globalAlpha = 0.4;
        ctx.beginPath(); ctx.moveTo(latestCandleX, pad.top); ctx.lineTo(latestCandleX, height - pad.bottom); ctx.stroke();
        ctx.globalAlpha = 1;
      }
      ctx.fillStyle = "#91a0bd";
      ctx.textAlign = "center";
      [minTime, (minTime + maxTime) / 2, maxTime].forEach((time) => {
        const x = useTime ? xForTime(time) : pad.left;
        ctx.fillText(shortDate(time), x, height - 10);
      });
      ctx.textAlign = "left";
      stairsChartState = { points: chartPoints, rect };
    };

    let stairsTooltipHideTimer = null;
    const hideStairsTooltip = () => {
      clearTimeout(stairsTooltipHideTimer);
      stairsTooltipHideTimer = setTimeout(() => {
        if (!stairsChartTooltip.matches(":hover")) stairsChartTooltip.style.display = "none";
      }, 100);
    };
    stairsChartTooltip.addEventListener("mouseenter", () => clearTimeout(stairsTooltipHideTimer));
    stairsChartTooltip.addEventListener("mouseleave", hideStairsTooltip);
    stairsChartTooltip.addEventListener("click", (event) => {
      const button = event.target.closest(".copy-point");
      if (!button || !stairsTooltipEvent) return;
      const payload = JSON.stringify({
        symbol_key: DATA.symbol_key,
        copied_at: new Date().toISOString(),
        point: stairsTooltipEvent,
        nearest_candle: eventCandle(stairsTooltipEvent),
      }, null, 2);
      copyToClipboard(payload).then(() => {
        const original = button.textContent;
        button.textContent = "Copied point JSON";
        setTimeout(() => { button.textContent = original; }, 1400);
      });
    });
    stairsChartWrap.addEventListener("scroll", hideStairsTooltip);
    stairsChart.addEventListener("mousemove", (event) => {
      if (!stairsChartState) return;
      const rect = stairsChart.getBoundingClientRect();
      const mouseX = event.clientX - rect.left;
      const mouseY = event.clientY - rect.top;
      const nearest = stairsChartState.points.reduce((best, point) => {
        const candidates = [
          { x: point.x, y: point.y },
          Number.isFinite(point.targetY) ? { x: point.targetX, y: point.targetY } : null,
          Number.isFinite(point.outcomeX) && Number.isFinite(point.outcomeY)
            ? { x: point.outcomeX, y: point.outcomeY } : null,
        ].filter(Boolean);
        const distance = Math.min(...candidates.map((candidate) =>
          Math.hypot(candidate.x - mouseX, candidate.y - mouseY)));
        return !best || distance < best.distance ? { point, distance } : best;
      }, null);
      if (!nearest || nearest.distance > 20) { hideStairsTooltip(); return; }
      const eventData = nearest.point.event;
      stairsTooltipEvent = eventData;
      const outcome = stairsOutcome(eventData);
      const expected = eventData.target_line == null ? "not available" : stairsLineName(eventData.target_line);
      const expectedPrice = eventData.expected_price_low != null
        ? price(eventData.expected_price_low) + " .. " + price(eventData.expected_price_high)
        : "not available";
      const positionTrade = positionTradeForEvent(eventData);
      const positionPnl = hasPositionPnl(eventData)
        ? signedPercent(eventData.position_pnl_pct) + " · " + (eventData.position_exit_reason || "closed")
        : positionTrade
          ? signedPercent(positionTrade.pnl_pct) + " · " + (positionTrade.exit_reason || "closed")
          : "no completed selected stair position";
      const structure = [
        eventData.staircase_lifecycle || "lifecycle unavailable",
        eventData.staircase_transition || "transition unavailable",
        "protected " + (eventData.staircase_protected_line == null ? "unavailable" : stairsLineName(eventData.staircase_protected_line)),
        "extreme " + (eventData.staircase_extreme_line == null ? "unavailable" : stairsLineName(eventData.staircase_extreme_line)),
      ].join(" · ");
      stairsChartTooltip.innerHTML = '<b>' + esc(eventData.pattern_key) + '</b> · ' + esc(date(eventData.time_ms)) +
        '<br><b>Detection:</b> ' + esc(stairDirection(eventData)) + ' at ' + esc(stairsLineName(eventData.line)) +
        ' · steps=' + esc(eventData.pattern_steps) + ' · sequence=' + esc(stairsSequence(eventData)) +
        '<br><b>Active stair primary:</b> ' + esc(activeStairSummary(eventData)) +
        (eventData.active_stair_wait_reason ? ' · ' + esc(eventData.active_stair_wait_reason) : '') +
        '<br><b>Active expectation:</b> ' + esc(eventData.active_stair_expected_line == null
          ? 'unavailable' : stairsLineName(eventData.active_stair_expected_line)) +
        (eventData.active_stair_route_available ? ' · route available' : ' · not executable yet') +
        '<br><b>Expectation:</b> ' + esc(expected) + ' · price range ' + esc(expectedPrice) +
        '<br><b>Structure:</b> ' + esc(structure) +
        '<br><b>Replay:</b> <span class="' + outcome.className + '">' + esc(outcome.label) + '</span>' +
        (eventData.prediction_resolved_at ? ' · resolved ' + esc(date(eventData.prediction_resolved_at)) : '') +
        '<br><b>Position PnL:</b> ' + esc(positionPnl) +
        '<br><b>Stair movement:</b> net ' + esc(signedPercent(eventData.stair_price_move_pct)) +
        ' · favorable ' + esc(signedPercent(eventData.stair_favorable_move_pct)) +
        ' · adverse ' + esc(signedPercent(eventData.stair_adverse_move_pct)) +
        '<br><button type="button" class="copy-point">Copy point JSON</button>';
      stairsChartTooltip.style.display = "block";
      const tooltipWidth = stairsChartTooltip.offsetWidth;
      const tooltipHeight = stairsChartTooltip.offsetHeight;
      const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
      const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
      stairsChartTooltip.style.left = Math.max(8, Math.min(event.clientX + 14, viewportWidth - tooltipWidth - 8)) + "px";
      stairsChartTooltip.style.top = Math.max(8, Math.min(event.clientY + 14, viewportHeight - tooltipHeight - 8)) + "px";
    });
    stairsChart.addEventListener("mouseleave", hideStairsTooltip);

    const renderPatternLegend = () => {
      document.getElementById("patterns-pnl-legend").innerHTML = parallelSeries.map((series) =>
        '<span class="legend-item"><span class="legend-swatch" style="background:' + series.color + '"></span>' +
        '<code>' + esc(series.pattern) + '</code> ' + money(series.summary.total_pnl_pct) +
        ' (' + (series.summary.trades || series.points.length - 1) + ' trades, MFE ' +
        valueRange(series.trades.map((trade) => trade.mfe_pct)) + ', MAE ' +
        valueRange(series.trades.map((trade) => trade.mae_pct)) + ')</span>'
      ).join("") || '<span class="subtle">No selected patterns with completed trades</span>';
    };
    const refreshPatternDisplay = () => {
      updatePatternSelection();
      refreshSelectedData();
      parallelSeries = buildParallelSeries();
      renderCards();
      renderCrossings();
      renderPatternTables();
      renderPatternReasons();
      renderPatternLegend();
      renderPositions();
      renderPredictions();
      drawChart();
      drawAllPatternChart();
      drawStairsChart();
    };
    patternFilter.addEventListener("click", (event) => {
      const chip = event.target.closest(".pattern-chip");
      if (!chip) return;
      chip.classList.toggle("selected");
      chip.setAttribute("aria-pressed", chip.classList.contains("selected") ? "true" : "false");
      refreshPatternDisplay();
    });
    document.getElementById("select-all-patterns").addEventListener("click", () => {
      [...patternFilter.querySelectorAll(".pattern-chip")].forEach((chip) => {
        chip.classList.add("selected");
        chip.setAttribute("aria-pressed", "true");
      });
      refreshPatternDisplay();
    });
    document.getElementById("clear-patterns").addEventListener("click", () => {
      [...patternFilter.querySelectorAll(".pattern-chip")].forEach((chip) => {
        chip.classList.remove("selected");
        chip.setAttribute("aria-pressed", "false");
      });
      refreshPatternDisplay();
    });
    renderPatternLegend();

    const renderPositions = () => {
      const visibleTrades = latestRows(selectedTrades);
      tableWindowNote("positions-note", selectedTrades.length, "positions");
      const positionRows = visibleTrades.map((trade, index) => {
        const sourceIndex = selectedTrades.length - visibleTrades.length + index;
        const side = trade.side > 0 ? "LONG" : "SHORT";
        const route = side + " " + trade.source_name + " → " + trade.target_name;
        const pnlClass = signedClass(trade.pnl_pct);
        return '<tr>' +
          '<td>' + (sourceIndex + 1) + ' ' + shareMarkup("trade", trade) + ' ' +
          reasonMarkup("position", sourceIndex, "", 16) + '</td>' +
          '<td>' + esc(side) + '</td>' +
          '<td>' + esc(route) + '</td>' +
          '<td>' + esc(date(trade.opened_at)) + '</td>' +
          '<td>' + price(trade.entry_price) + '</td>' +
          '<td>' + price(trade.target_price) + '</td>' +
          '<td>' + price(trade.stop_price) + '</td>' +
          '<td>' + esc(date(trade.closed_at)) + '</td>' +
          '<td>' + price(trade.exit_price) + '</td>' +
          '<td>' + esc(trade.exit_reason) + '</td>' +
          '<td>' + money(trade.gross_pnl_pct) + '</td>' +
          '<td class="negative">-' + money(Number(trade.entry_fee_pct || 0) + Number(trade.exit_fee_pct || 0)) + '</td>' +
          '<td class="' + pnlClass + '">' + money(trade.pnl_pct) + '</td>' +
          '<td>' + money(trade.mfe_pct) + '</td>' +
          '<td>' + money(trade.mae_pct) + '</td>' +
          '<td>' + duration(trade.opened_at, trade.closed_at) + '</td>' +
          '</tr>';
      });
      if (selectedOpenTrade) {
        const trade = selectedOpenTrade;
        const side = trade.side > 0 ? "LONG" : "SHORT";
        const route = side + " " + trade.source_name + " → " + trade.target_name;
        positionRows.push(
          '<tr class="open"><td>open ' + shareMarkup("trade", trade) + '</td><td>' + esc(side) + '</td><td>' + esc(route) +
          '</td><td>' + esc(date(trade.opened_at)) + '</td><td>' + price(trade.entry_price) +
          '</td><td>' + price(trade.target_price) + '</td><td>' + price(trade.stop_price) +
          '</td><td>—</td><td>—</td><td>OPEN</td><td>—</td><td>—</td><td>—</td><td>—</td><td>—</td></tr>' +
          '<tr class="trade-reason"><td colspan="16"><span class="trade-reason-label">Entry reason:</span>' +
          formatReason(trade.reason) + '</td></tr>'
        );
      }
      document.getElementById("positions").innerHTML = positionRows.join("") ||
        '<tr><td colspan="16" class="subtle">No selected positions</td></tr>';
    };

    const renderPredictions = () => {
      const visiblePredictions = latestRows(selectedPredictions);
      tableWindowNote("predictions-note", selectedPredictionTotal(), "predictions", selectedPredictions.length);
      const predictionRows = visiblePredictions.map((prediction, index) => {
      const sourceIndex = selectedPredictions.length - visiblePredictions.length + index;
      const side = prediction.side > 0 ? "LONG" : "SHORT";
      const route = side + " " + prediction.source_name + " → " + prediction.target_name;
      const outcomeClass = prediction.outcome === "prediction_broken" ? "negative"
        : prediction.outcome === "unresolved" ? "" : "positive";
      return '<tr>' +
        '<td>' + (sourceIndex + 1) + ' ' + shareMarkup("prediction", prediction) + ' ' +
        reasonMarkup("prediction", sourceIndex, "", 13) + '</td><td>' + esc(side) + '</td><td>' + esc(route) + '</td>' +
        '<td>' + esc(date(prediction.created_at)) + '</td><td>' + price(prediction.entry_price) + '</td>' +
        '<td>' + price(prediction.target_price) + '</td><td>' + price(prediction.stop_price) + '</td>' +
        '<td>' + esc(date(prediction.resolved_at)) + '</td><td>' + (prediction.filled ? "yes" : "no") + '</td>' +
        '<td>' + (prediction.furthest_line_before_return_name
          ? esc(prediction.furthest_line_before_return_name + " (#" + prediction.furthest_line_before_return + ")")
          : "—") + '</td><td>' + (prediction.returned_to_source_line ? "yes" : "no") + '</td>' +
        '<td class="' + outcomeClass + '">' + esc(prediction.outcome) + '</td>' +
        '<td>' + duration(prediction.created_at, prediction.resolved_at) + '</td></tr>';
    });
    document.getElementById("predictions").innerHTML = predictionRows.join("") ||
      '<tr><td colspan="13" class="subtle">No selected predictions</td></tr>';
    };

    function drawChart() {
      const canvas = document.getElementById("pnl-chart");
      const empty = document.getElementById("chart-empty");
      empty.style.display = selectedTrades.length ? "none" : "grid";
      if (!selectedTrades.length) {
        canvas.getContext("2d").clearRect(0, 0, canvas.width, canvas.height);
        return;
      }
      const rect = canvas.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.max(1, Math.floor(rect.width * ratio));
      canvas.height = Math.max(1, Math.floor(rect.height * ratio));
      const ctx = canvas.getContext("2d");
      ctx.scale(ratio, ratio);
      const width = rect.width, height = rect.height;
      const pad = { left: 58, right: 18, top: 18, bottom: 34 };
      const values = [0];
      selectedTrades.forEach((trade) => values.push(values[values.length - 1] + Number(trade.pnl_pct || 0)));
      let min = Math.min(...values), max = Math.max(...values);
      if (min === max) { min -= 1; max += 1; }
      const span = max - min;
      min -= span * 0.08; max += span * 0.08;
      const x = (index) => pad.left + index / (values.length - 1) *
        (width - pad.left - pad.right);
      const y = (value) => pad.top + (max - value) / (max - min) *
        (height - pad.top - pad.bottom);
      ctx.clearRect(0, 0, width, height);
      ctx.font = "12px system-ui";
      ctx.strokeStyle = "#2c3a5a";
      ctx.fillStyle = "#91a0bd";
      ctx.lineWidth = 1;
      for (let i = 0; i <= 4; i++) {
        const value = min + (max - min) * i / 4;
        const lineY = y(value);
        ctx.beginPath(); ctx.moveTo(pad.left, lineY); ctx.lineTo(width - pad.right, lineY);
        ctx.stroke();
        ctx.fillText(value.toFixed(2) + "%", 4, lineY + 4);
      }
      ctx.strokeStyle = "#66789e";
      ctx.setLineDash([5, 4]);
      ctx.beginPath(); ctx.moveTo(pad.left, y(0)); ctx.lineTo(width - pad.right, y(0)); ctx.stroke();
      ctx.setLineDash([]);
      ctx.strokeStyle = values[values.length - 1] >= 0 ? "#55d69b" : "#ff7185";
      ctx.lineWidth = 2.5;
      ctx.beginPath();
      values.forEach((value, index) => {
        if (index === 0) ctx.moveTo(x(index), y(value));
        else ctx.lineTo(x(index), y(value));
      });
      ctx.stroke();
      values.forEach((value, index) => {
        ctx.fillStyle = index === 0 || selectedTrades[index - 1].pnl_pct >= 0 ? "#55d69b" : "#ff7185";
        ctx.beginPath(); ctx.arc(x(index), y(value), 3.5, 0, Math.PI * 2); ctx.fill();
      });
      ctx.fillStyle = "#91a0bd";
      ctx.textAlign = "center";
      const labelIndexes = [...new Set([0, Math.floor((values.length - 1) / 2), values.length - 1])];
      labelIndexes.forEach((index) => {
        const time = index === 0
          ? selectedTrades[0].opened_at
          : selectedTrades[index - 1] && selectedTrades[index - 1].closed_at;
        ctx.fillText(shortDate(time), x(index), height - 8);
      });
      ctx.textAlign = "left";
    }
    window.addEventListener("resize", drawChart);
    window.addEventListener("resize", drawAllPatternChart);
    window.addEventListener("resize", drawStairsChart);
    refreshPatternDisplay();
  </script>
</body>
</html>
"###;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::virtual_trade::{VirtualPredictionSummary, VirtualTradeSummary};
    use serde_json::json;

    #[test]
    fn generated_report_is_standalone_html() {
        let report = AnalysisReport {
            bypass_entry_blockers: false,
            bypass_forced_candidates: 0,
            bypass_forced_setups: 0,
            bypass_forced_market_accepted: 0,
            bypass_forced_market_busy: 0,
            crossing_count: 0,
            pattern_count: 0,
            patterns: Vec::new(),
            latest_event: None,
            forecast: super::super::mrc_crossing::Forecast {
                source_line: None,
                source_name: None,
                condition: "no-crossing-history",
                sample_count: 0,
                estimated_time_ms: None,
                likely_destinations: Vec::new(),
            },
            events: Vec::new(),
            trade_summary: VirtualTradeSummary::default(),
            trades: Vec::new(),
            open_trade: None,
            prediction_summary: VirtualPredictionSummary::default(),
            predictions: Vec::new(),
            pending_predictions: 0,
            active_predictions: 0,
            pattern_win_rates: Vec::new(),
            stair_statistics: super::super::mrc_crossing::StairStatistics::default(),
            pattern_performance: Vec::new(),
            parallel_pattern_trades: Vec::new(),
            active_pattern: None,
            active_pattern_score: 0.0,
        };
        let data = json!({
            "symbol_key": "TEST",
            "history_requested_1m": 10,
            "history_loaded_1m": 10,
            "trade_summary": report.trade_summary,
            "trades": report.trades,
            "open_trade": report.open_trade,
            "prediction_summary": report.prediction_summary,
            "predictions": report.predictions,
        });
        let html = TEMPLATE.replace("__DATA__", &serde_json::to_string(&data).unwrap());
        assert!(html.contains("<canvas id=\"pnl-chart\"></canvas>"));
        assert!(html.contains("<canvas id=\"patterns-pnl-chart\"></canvas>"));
        assert!(html.contains("<canvas id=\"stairs-chart\"></canvas>"));
        assert!(html.contains("Stairs detection / expectation"));
        assert!(html.contains("dashed arrow/band = expectation"));
        assert!(html.contains("protected/extreme ticks = stair structure"));
        assert!(html.contains("MRC mean"));
        assert!(html.contains("bull candle"));
        assert!(html.contains("outer ring = active stair primary"));
        assert!(html.contains("Active stair primary"));
        assert!(html.contains("active_stair_expected_line"));
        assert!(html.contains("followed (correct)"));
        assert!(html.contains("broken (wrong)"));
        assert!(html.contains("id=\"pattern-filter\""));
        assert!(html.contains("class=\"pattern-chip selected\""));
        assert!(html.contains("class=\"copy-id\""));
        assert!(html.contains("class=\"copy-point\""));
        assert!(html.contains("navigator.clipboard"));
        assert!(html.contains("line plans"));
        assert!(html.contains("covered_by_target"));
        assert!(html.contains("line-plan-table"));
        assert!(html.contains("crossing-detail"));
        assert!(html.contains("crossing-route"));
        assert!(html.contains("Current prediction"));
        assert!(html.contains("Prediction / long result"));
        assert!(html.contains("Possible move before SL"));
        assert!(html.contains("position_mfe_pct"));
        assert!(html.contains("max-width: 1880px"));
        assert!(html.contains(".crossing-panel .table-wrap { overflow-x: visible; }"));
        assert!(html.contains(".prediction-lock-note { display: block;"));
        assert!(html.contains("box-sizing: border-box"));
        assert!(
            html.contains(".crossing-table { width: 100%; min-width: 0; table-layout: fixed; }")
        );
        assert!(!html.contains("min-width: 760px"));
        assert!(!html.contains("Line probabilities / big break"));
        assert!(html.contains(">All</button>"));
        assert!(html.contains("Clear"));
        assert!(html.contains("Positions"));
        assert!(html.contains("Entry reason:"));
        assert!(html.contains("Cumulative PnL"));
        assert!(html.contains("All-pattern PnL comparison"));
        assert!(html.contains("Reasons for all pattern trades"));
        assert!(html.contains("Volume gain"));
        assert!(html.contains("Buy/sell flow"));
        assert!(html.contains("MFE price move"));
        assert!(html.contains("MAE price move"));
        assert!(html.contains("Prediction outcomes"));
        assert!(html.contains("Win rate by pattern"));
    }
}
