use std::error::Error;
use std::fs;

use serde_json::json;

use super::mrc_crossing::AnalysisReport;

pub(crate) fn write_report(
    path: &str,
    symbol_key: &str,
    history_requested: usize,
    minute_count: usize,
    direction: Option<i8>,
    report: &AnalysisReport,
) -> Result<(), Box<dyn Error>> {
    let data = json!({
        "symbol_key": symbol_key,
        "history_requested_1m": history_requested,
        "history_loaded_1m": minute_count,
        "direction": direction.map(|value| if value > 0 { "long" } else { "short" }),
        "latest_event_time_ms": report.latest_event.as_ref().map(|event| event.time_ms),
        "trade_summary": report.trade_summary,
        "trades": report.trades,
        "open_trade": report.open_trade,
        "prediction_summary": report.prediction_summary,
        "predictions": report.predictions,
        "pending_predictions": report.pending_predictions,
        "active_predictions": report.active_predictions,
        "pattern_win_rates": report.pattern_win_rates,
        "stair_statistics": report.stair_statistics,
        "pattern_performance": report.pattern_performance,
        "parallel_pattern_trades": report.parallel_pattern_trades,
        "active_pattern": report.active_pattern,
        "active_pattern_score": report.active_pattern_score,
        "pattern_definitions": {
            "pattern": "A stable local movement classification: staircase, break-return, range/trend cycle, pullback, or outward continuation.",
            "pattern_steps": "For a staircase, the number of same-direction zigzag legs supporting the classified pattern.",
            "stair_sequence": "The MRC line sequence used by the STAIRS_UP or STAIRS_DOWN decision.",
            "stair_price_move_pct": "Direction-adjusted net price movement from the beginning of the stair to the current event.",
            "stair_favorable_move_pct": "Maximum favorable price excursion from the beginning of the stair.",
            "stair_adverse_move_pct": "Maximum adverse price excursion from the beginning of the stair.",
            "staircase_lifecycle": "Current staircase state: progressing, pullback, stalled, or invalidated.",
            "staircase_transition": "Structural transition that produced the current staircase state.",
            "staircase_protected_line": "Protected swing line. Breaking it invalidates the staircase.",
            "staircase_extreme_line": "Furthest line reached by the latest confirmed directional leg.",
            "cascade_phase": "Current candle-cascade phase: starting, building, mature, counter_move, or reversal.",
            "cascade_potential_move_pct": "Continuation range estimated from the active and recent same-direction cascade movements.",
            "cascade_adverse_move_pct": "Largest recent opposite-direction cascade movement, used as adverse-excursion risk.",
            "cascade_trade_state": "Cascade execution gate explaining whether direction, speed, potential, and adverse risk permit entry.",
            "outcome_counts": "Exact rejection, continuation, and neutral outcome counts used by the classifier. They are raw evidence, not trade win rate.",
            "resistance": "The opposite reverted MRC line and its barrier state at the signal.",
            "tp_sl": "TP and SL are the actual MRC lines used by the executed side, not merely the learned destination.",
            "trace_outcome": "Observed movement is profitable when its favorable excursion is larger than its absolute adverse excursion; otherwise it is negative.",
            "position_mfe": "For a completed filled position, MFE is the maximum favorable price movement observed before the simulated target, stop, or end-of-history exit. OHLC data cannot determine intrabar order when target and stop share one candle.",
            "win_rate": "Wins divided by closed virtual positions for the exact displayed pattern; the position count must be read with the percentage."
        },
        "events": report.events,
        "forecast": report.forecast,
    });
    let payload = serde_json::to_string(&data)?.replace('<', "\\u003c");
    let html = TEMPLATE.replace("__DATA__", &payload);
    fs::write(path, html)?;
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
    canvas { width: 100%; height: 100%; display: block; }
    .chart-tooltip {
      position: absolute; z-index: 2; display: none; max-width: 420px;
      padding: 10px 12px; background: #0b1020; border: 1px solid var(--border);
      border-radius: 8px; box-shadow: 0 8px 24px rgba(0, 0, 0, .35);
      pointer-events: none; font-size: 12px;
    }
    .chart-legend { display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 12px; }
    .legend-item { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); font-size: 12px; }
    .legend-swatch { width: 12px; height: 3px; border-radius: 2px; }
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
    .copy-id {
      color: var(--muted); background: var(--panel-2); border: 1px solid var(--border);
      border-radius: 6px; padding: 3px 6px; cursor: pointer; font: inherit; font-size: 11px;
    }
    .copy-id:hover { color: var(--text); border-color: var(--blue); }
    .empty {
      position: absolute; inset: 0; display: grid; place-items: center;
      color: var(--muted); pointer-events: none;
    }
    .table-wrap { overflow-x: auto; }
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
      <h2>Aggregated stair statistics</h2>
      <p class="subtle">This aggregates every detected zigzag staircase, not only filled positions. Observations count crossing detections; prediction matches include forecast evaluations; selected-position PnL is the actual account route for this run.</p>
      <div id="stair-statistics" class="table-wrap"></div>
    </section>

    <section class="panel">
      <h2>Positions</h2>
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
      <p class="subtle">Long-only execution focus. Each crossing shows only its current prediction result, target/SL movement, and filled long-position result when available.</p>
      <div id="crossings" class="table-wrap"></div>
    </section>
  </main>

  <script>
    const DATA = __DATA__;
    const trades = DATA.trades || [];
    const tradeSummary = DATA.trade_summary || {};
    const predictions = DATA.predictions || [];
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
      const target = event.take_profit_name || event.target_name || "not selected";
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
    const summarizePredictions = (rows) => {
      const followed = rows.filter((prediction) =>
        prediction.outcome === "reached_and_followed" || prediction.outcome === "not_reached_but_followed").length;
      return {
        predictions: rows.length,
        filled: rows.filter((prediction) => prediction.filled).length,
        followedRate: rows.length ? followed / rows.length * 100 : 0,
      };
    };
    const renderCards = () => {
      const tradeStats = summarizeTrades(selectedTrades);
      const predictionStats = summarizePredictions(selectedPredictions);
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
      const crossingRows = selectedEvents.slice(-500).reverse().map((event) => {
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
          " protected=" + (event.staircase_protected_line != null ? lineLabels[event.staircase_protected_line] : "na") +
          " extreme=" + (event.staircase_extreme_line != null ? lineLabels[event.staircase_extreme_line] : "na") +
          " cascade_state=" + (event.cascade_trade_state || "not_configured") +
          " cascade_dir=" + (event.cascade_direction > 0 ? "long" : event.cascade_direction < 0 ? "short" : "na") +
          " cascade_potential=" + (event.cascade_potential_move_pct == null ? "na" : Number(event.cascade_potential_move_pct).toFixed(3) + "%") +
          " cascade_adverse=" + (event.cascade_adverse_move_pct == null ? "na" : Number(event.cascade_adverse_move_pct).toFixed(3) + "%")
        : "";
      const compactResult = predictionSummary(event) +
        (event.position_outcome ? '<br><span class="subtle">LONG ' + positionSummary(event) + '</span>' : '');
      return '<tr><td class="numeric">' + esc(date(event.time_ms)) + ' ' + shareMarkup("event", event) + '</td><td>' + direction +
        ' ' + esc(event.line_name) + '</td><td class="numeric">' + price(event.market_price) +
        '</td><td class="numeric">' + price(event.effective_entry_price) +
        '</td><td>' + esc(learned) + '</td><td>' + esc(event.trade_direction > 0 ? "LONG" : event.trade_direction < 0 ? "SHORT" : "—") +
        '</td><td>' + esc(exits) + '<br><span class="subtle">' + esc(resistance) + '</span></td>' +
        '<td>' + compactResult + '</td></tr>' +
        '<tr class="crossing-detail"><td colspan="8"><div class="crossing-detail-content">' +
          linePlanDetails(event) +
          '<div class="crossing-evidence">' +
            '<span class="crossing-evidence-label">Historical evidence</span><span>' + esc(evidence + (stair ? " " + stair : "")) + '</span>' +
          '</div>' +
        '</div></td></tr>';
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

    const patternColors = ["#72a7ff", "#55d69b", "#ff7185", "#d7a6ff", "#ffbf69", "#6ee7e7", "#f08cae"];
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
        const rows = groupTrades.map((trade, index) => {
          const side = Number(trade.side) > 0 ? "LONG" : "SHORT";
          const fees = Number(trade.entry_fee_pct || 0) + Number(trade.exit_fee_pct || 0);
          const pnlClass = signedClass(trade.pnl_pct);
          const route = side + " " + trade.source_name + " → " + trade.target_name;
          return '<tr><td>' + (index + 1) + ' ' + shareMarkup("trade", trade) + '</td><td>' + esc(date(trade.closed_at)) +
            '</td><td>' + esc(side) + '</td><td>' + esc(route) + '</td><td>' +
            price(trade.entry_price) + '</td><td>' + price(trade.target_price) + '</td><td>' +
            price(trade.stop_price) + '</td><td>' + esc(trade.exit_reason) + '</td><td>' +
            money(trade.gross_pnl_pct) + '</td><td class="negative">-' + money(fees) +
            '</td><td class="' + pnlClass + '">' + money(trade.pnl_pct) + '</td><td>' +
            money(trade.mfe_pct) + '</td><td>' + money(trade.mae_pct) + '</td></tr>' +
            '<tr class="trade-reason"><td colspan="13"><span class="trade-reason-label">Reason:</span> ' +
            formatReason(trade.reason) + '</td></tr>';
        }).join("");
        const net = money(summary.total_pnl_pct);
        const mfeRange = valueRange(groupTrades.map((trade) => trade.mfe_pct));
        const maeRange = valueRange(groupTrades.map((trade) => trade.mae_pct));
        return '<details class="pattern-reason-group" open><summary><code>' +
          esc(group.pattern) + '</code> · ' + groupTrades.length + ' closed · net ' +
          '<span class="' + signedClass(summary.total_pnl_pct) + '">' + net +
          '</span> · MFE ' + mfeRange + ' · MAE ' + maeRange + '</summary>' + (rows
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
      const positionRows = selectedTrades.map((trade, index) => {
        const side = trade.side > 0 ? "LONG" : "SHORT";
        const route = side + " " + trade.source_name + " → " + trade.target_name;
        const pnlClass = signedClass(trade.pnl_pct);
        return '<tr>' +
          '<td>' + (index + 1) + ' ' + shareMarkup("trade", trade) + '</td>' +
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
          '</tr>' +
          '<tr class="trade-reason"><td colspan="16"><span class="trade-reason-label">Entry reason:</span>' +
          formatReason(trade.reason) + '</td></tr>';
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
      const predictionRows = selectedPredictions.map((prediction, index) => {
      const side = prediction.side > 0 ? "LONG" : "SHORT";
      const route = side + " " + prediction.source_name + " → " + prediction.target_name;
      const outcomeClass = prediction.outcome === "prediction_broken" ? "negative"
        : prediction.outcome === "unresolved" ? "" : "positive";
      return '<tr>' +
        '<td>' + (index + 1) + ' ' + shareMarkup("prediction", prediction) + '</td><td>' + esc(side) + '</td><td>' + esc(route) + '</td>' +
        '<td>' + esc(date(prediction.created_at)) + '</td><td>' + price(prediction.entry_price) + '</td>' +
        '<td>' + price(prediction.target_price) + '</td><td>' + price(prediction.stop_price) + '</td>' +
        '<td>' + esc(date(prediction.resolved_at)) + '</td><td>' + (prediction.filled ? "yes" : "no") + '</td>' +
        '<td>' + (prediction.furthest_line_before_return_name
          ? esc(prediction.furthest_line_before_return_name + " (#" + prediction.furthest_line_before_return + ")")
          : "—") + '</td><td>' + (prediction.returned_to_source_line ? "yes" : "no") + '</td>' +
        '<td class="' + outcomeClass + '">' + esc(prediction.outcome) + '</td>' +
        '<td>' + duration(prediction.created_at, prediction.resolved_at) + '</td></tr>' +
        '<tr class="trade-reason"><td colspan="13"><span class="trade-reason-label">Prediction reason:</span>' +
        formatReason(prediction.reason) + '</td></tr>';
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
    refreshPatternDisplay();
  </script>
</body>
</html>
"###;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::virtual_trade::{VirtualPredictionSummary, VirtualTradeSummary};

    #[test]
    fn generated_report_is_standalone_html() {
        let report = AnalysisReport {
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
        assert!(html.contains("id=\"pattern-filter\""));
        assert!(html.contains("class=\"pattern-chip selected\""));
        assert!(html.contains("class=\"copy-id\""));
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
