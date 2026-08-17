use std::error::Error;
use std::fs;

use serde_json::json;

use super::mrc_crossing::AnalysisReport;

pub(crate) fn write_report(
    path: &str,
    symbol_key: &str,
    history_requested: usize,
    minute_count: usize,
    hourly_count: usize,
    report: &AnalysisReport,
) -> Result<(), Box<dyn Error>> {
    let data = json!({
        "symbol_key": symbol_key,
        "history_requested_1m": history_requested,
        "history_loaded_1m": minute_count,
        "history_loaded_1h": hourly_count,
        "latest_event_time_ms": report.latest_event.as_ref().map(|event| event.time_ms),
        "trade_summary": report.trade_summary,
        "trades": report.trades,
        "open_trade": report.open_trade,
        "prediction_summary": report.prediction_summary,
        "predictions": report.predictions,
        "pending_predictions": report.pending_predictions,
        "active_predictions": report.active_predictions,
        "pattern_win_rates": report.pattern_win_rates,
        "pattern_definitions": {
            "pattern": "A stable local movement classification: staircase, break-return, range/trend cycle, pullback, or outward continuation.",
            "pattern_steps": "The number of local line steps supporting the classified pattern.",
            "probability": "Supporting route observations for the selected pattern target. It is evidence, not the direction decision and not trade win rate.",
            "resistance": "The opposite reverted MRC line and its barrier state at the signal.",
            "tp_sl": "TP and SL are the actual MRC lines used by the executed side, not merely the learned destination.",
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
    .page { max-width: 1400px; margin: 0 auto; padding: 28px; }
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
    canvas { width: 100%; height: 100%; display: block; }
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
    .positive { color: var(--green); }
    .negative { color: var(--red); }
    .open { color: var(--blue); }
    code { color: var(--text); white-space: normal; }
    @media (max-width: 700px) {
      .page { padding: 16px; }
      .chart-wrap { height: 260px; }
    }
  </style>
</head>
<body>
  <main class="page">
    <section class="header">
      <h1>MRC Crossing PnL Report</h1>
      <div id="context" class="subtle"></div>
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
      <h2>Positions</h2>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>#</th><th>Side</th><th>Route</th><th>Opened</th>
              <th>Entry</th><th>Target</th><th>Stop</th><th>Closed</th>
              <th>Exit</th><th>Outcome</th><th>Gross PnL</th><th>Fees</th><th>Net PnL</th>
              <th>Duration</th>
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
        <p><b>Pattern steps</b> and elapsed time describe how much local evidence is currently visible. <b>Probability</b> is supporting route evidence; the pattern itself selects the direction.</p>
        <p><b>Resistance</b> is the opposite reverted MRC line and its current barrier state. <b>TP / SL</b> are the actual MRC lines used by the executed side.</p>
        <p><b>Max/furthest line before return</b> is the exact MRC line index/name farthest from the source line before the first return; it does not use the changing line price.</p>
        <p><b>Win rate</b> is wins divided by closed virtual positions for the exact displayed pattern. Always read it together with the position count.</p>
      </div>
    </section>

    <section class="panel">
      <h2>Win rate by pattern</h2>
      <div id="patterns" class="table-wrap"></div>
    </section>

    <section class="panel">
      <h2>Crossing / price trace</h2>
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
    const esc = (value) => String(value ?? "")
      .replaceAll("&", "&amp;").replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;").replaceAll('"', "&quot;");
    const money = (value) => Number(value || 0).toFixed(4) + "%";
    const price = (value) => Number(value || 0).toFixed(6);
    const date = (value) => value ? new Date(Number(value)).toLocaleString() : "—";
    const shortDate = (value) => value ? new Date(Number(value)).toLocaleString([], {
      month: "short", day: "numeric", hour: "2-digit", minute: "2-digit"
    }) : "—";
    const duration = (opened, closed) => {
      if (!opened || !closed) return "—";
      const minutes = (Number(closed) - Number(opened)) / 60000;
      return minutes.toFixed(2) + "m";
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
          ["Pattern elapsed", "pattern_elapsed_ms"],
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
        ["Resistance", [
          ["Barrier state / line", "barrier"],
          ["Resistance line / price", "resistance_line", null, "level"],
          ["Reverted line", "reverted_line"],
        ]],
        ["Historical evidence", [
          ["History scope", "history_scope"],
          ["Probability (selected / observations)", "probability"],
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
          ["Estimated time", "eta_ms"],
          ["Projection", "projection"],
        ]],
      ];
      const sectionHtml = sections.map(([title, fields]) => {
        const items = fields.map(([label, firstKey, secondKey, valueType]) => {
          if (values[firstKey] == null && (!secondKey || values[secondKey] == null)) return "";
          const value = valueType === "level"
            ? formatLevel(firstKey, secondKey)
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
      " · generated from " + DATA.history_loaded_1h + " hourly candles" +
      (DATA.latest_event_time_ms ? " · as of " + date(DATA.latest_event_time_ms) : "");

    const cardData = [
      ["Positions", tradeSummary.trades || 0],
      ["Win rate", Number(tradeSummary.win_rate_pct || 0).toFixed(1) + "%"],
      ["Net PnL", Number(tradeSummary.total_pnl_pct || 0).toFixed(4) + "%"],
      ["Fees", Number(tradeSummary.total_fees_pct || 0).toFixed(4) + "%"],
      ["Max drawdown", Number(tradeSummary.max_drawdown_pct || 0).toFixed(4) + "%"],
      ["Avg duration", tradeSummary.average_duration_ms
        ? (Number(tradeSummary.average_duration_ms) / 60000).toFixed(2) + "m" : "—"],
      ["Predictions", summary.predictions || 0],
      ["Filled predictions", summary.filled || 0],
      ["Follow rate", Number(summary.followed_rate_pct || 0).toFixed(1) + "%"],
    ];
    document.getElementById("cards").innerHTML = cardData.map(([label, value]) =>
      '<div class="card"><div class="card-label">' + esc(label) +
      '</div><div class="card-value">' + esc(value) + '</div></div>'
    ).join("");

    const crossings = DATA.events || [];
    const crossingRows = crossings.slice(-500).reverse().map((event) => {
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
      const probability = event.target_samples + "/" + event.pattern_samples +
        " = " + (Number(event.probability || 0) * 100).toFixed(1) + "%";
      const expected = event.expected_price_low != null
        ? price(event.expected_price_low) + ".." + price(event.expected_price_high) +
          " (" + esc(event.direction > 0
            ? (event.expected_price_high > event.mrc_max ? "above_mrc_max"
              : event.expected_price_high < event.mrc_max ? "below_mrc_max" : "same_mrc_max")
            : (event.expected_price_low < event.mrc_min ? "below_mrc_min"
              : event.expected_price_low > event.mrc_min ? "above_mrc_min" : "same_mrc_min")) + ")"
        : "—";
      const realizedPrice = event.realized_price_low != null
        ? price(event.realized_price_low) + ".." + price(event.realized_price_high)
        : "—";
      const targetRange = event.realized_target_low != null
        ? price(event.realized_target_low) + ".." + price(event.realized_target_high)
        : "—";
      const evidence = event.target_line == null
        ? "no prediction"
        : "history=" + event.history_scope +
          " context=" + event.context_samples +
          " no_pullback=" + event.no_pullback_count +
          " reverted=" + (event.reverted_line_name || "na") +
          " next=" + event.next_line_crosses +
          " support/resistance=" + event.support_resistance_moves +
          " price-only=" + event.price_only_moves;
      return '<tr><td>' + esc(date(event.time_ms)) + '</td><td>' + direction +
        ' ' + esc(event.line_name) + '</td><td>' + price(event.market_price) +
        '</td><td>' + price(event.mrc_min) + '..' + price(event.mrc_max) +
        '</td><td>' + esc(learned) + '</td><td>' + esc(event.trade_direction > 0 ? "LONG" : "SHORT") +
        '</td><td>' + esc(exits) + '</td><td>' + esc(resistance) + '</td><td>' + probability +
        '</td><td>' + expected + '</td><td>' + realizedPrice + '</td><td>' + targetRange +
        '</td><td><code>' + esc(event.mrc_trace || "—") + '</code></td><td>' +
        esc(evidence) + '</td></tr>';
    }).join("");
    document.getElementById("crossings").innerHTML = crossings.length
      ? '<table><thead><tr><th>Time</th><th>Cross</th><th>Market price</th>' +
        '<th>MRC min..max</th><th>Prediction</th>' +
        '<th>Trade side</th><th>TP / SL lines</th><th>Resistance</th><th>Probability</th>' +
        '<th>Expected price</th><th>Observed price low..high</th><th>Target MRC low..high</th>' +
        '<th>MRC movement</th><th>Evidence</th></tr></thead><tbody>' + crossingRows + '</tbody></table>'
      : '<div class="subtle">No crossing observations</div>';

    const patternRows = patternWinRates.map((pattern) =>
      '<tr><td><code>' + esc(pattern.pattern) + '</code></td><td>' + pattern.matches + '</td><td>' +
      pattern.filled_predictions + '</td><td>' + pattern.closed_positions + '</td><td>' +
      pattern.wins + ' / ' + pattern.losses + '</td><td>' + Number(pattern.win_rate_pct || 0).toFixed(1) + '%</td><td>' +
      Number(pattern.average_probability_pct || 0).toFixed(1) + '% (' +
      Number(pattern.minimum_probability_pct || 0).toFixed(1) + '..' +
      Number(pattern.maximum_probability_pct || 0).toFixed(1) + '%)</td><td>' +
      pattern.followed_predictions + ' / ' + pattern.broken_predictions + '</td><td>' +
      money(pattern.total_gross_pnl_pct) + '</td><td>' + money(pattern.total_fees_pct) +
      '</td><td class="' + signedClass(pattern.total_pnl_pct) + '">' + money(pattern.total_pnl_pct) +
      '</td><td>' + money(pattern.average_win_pnl_pct) + '</td><td>' +
      money(pattern.average_loss_pnl_pct) + '</td></tr>'
    ).join("");
    document.getElementById("patterns").innerHTML = patternWinRates.length
      ? '<table><thead><tr><th>Pattern</th><th>Matches</th><th>Filled</th><th>Closed</th>' +
        '<th>Wins / losses</th><th>Win rate</th><th>Prediction probability avg (range)</th>' +
        '<th>Followed / broken</th><th>Gross</th><th>Fees</th><th>Net</th>' +
        '<th>Average win</th><th>Average loss</th></tr></thead><tbody>' + patternRows + '</tbody></table>'
      : '<div class="subtle">No matched patterns with completed positions</div>';

    const positionRows = trades.map((trade, index) => {
      const side = trade.side > 0 ? "LONG" : "SHORT";
      const route = side + " " + trade.source_name + " → " + trade.target_name;
      const pnlClass = signedClass(trade.pnl_pct);
      return '<tr>' +
        '<td>' + (index + 1) + '</td>' +
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
        '<td>' + duration(trade.opened_at, trade.closed_at) + '</td>' +
        '</tr>' +
        '<tr class="trade-reason"><td colspan="14"><span class="trade-reason-label">Entry reason:</span>' +
        formatReason(trade.reason) + '</td></tr>';
    });
    if (DATA.open_trade) {
      const trade = DATA.open_trade;
      const side = trade.side > 0 ? "LONG" : "SHORT";
      const route = side + " " + trade.source_name + " → " + trade.target_name;
      positionRows.push(
        '<tr class="open"><td>open</td><td>' + esc(side) + '</td><td>' + esc(route) +
        '</td><td>' + esc(date(trade.opened_at)) + '</td><td>' + price(trade.entry_price) +
        '</td><td>' + price(trade.target_price) + '</td><td>' + price(trade.stop_price) +
        '</td><td>—</td><td>—</td><td>OPEN</td><td>—</td><td>—</td><td>—</td><td>—</td></tr>' +
        '<tr class="trade-reason"><td colspan="14"><span class="trade-reason-label">Entry reason:</span>' +
        formatReason(trade.reason) + '</td></tr>'
      );
    }
    document.getElementById("positions").innerHTML = positionRows.join("") ||
      '<tr><td colspan="14" class="subtle">No positions</td></tr>';

    const predictionRows = predictions.map((prediction, index) => {
      const side = prediction.side > 0 ? "LONG" : "SHORT";
      const route = side + " " + prediction.source_name + " → " + prediction.target_name;
      const outcomeClass = prediction.outcome === "prediction_broken" ? "negative"
        : prediction.outcome === "unresolved" ? "" : "positive";
      return '<tr>' +
        '<td>' + (index + 1) + '</td><td>' + esc(side) + '</td><td>' + esc(route) + '</td>' +
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
      '<tr><td colspan="11" class="subtle">No predictions</td></tr>';

    function drawChart() {
      const canvas = document.getElementById("pnl-chart");
      const empty = document.getElementById("chart-empty");
      empty.style.display = trades.length ? "none" : "grid";
      if (!trades.length) return;
      const rect = canvas.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.max(1, Math.floor(rect.width * ratio));
      canvas.height = Math.max(1, Math.floor(rect.height * ratio));
      const ctx = canvas.getContext("2d");
      ctx.scale(ratio, ratio);
      const width = rect.width, height = rect.height;
      const pad = { left: 58, right: 18, top: 18, bottom: 34 };
      const values = [0];
      trades.forEach((trade) => values.push(values[values.length - 1] + Number(trade.pnl_pct || 0)));
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
        ctx.fillStyle = index === 0 || trades[index - 1].pnl_pct >= 0 ? "#55d69b" : "#ff7185";
        ctx.beginPath(); ctx.arc(x(index), y(value), 3.5, 0, Math.PI * 2); ctx.fill();
      });
      ctx.fillStyle = "#91a0bd";
      ctx.textAlign = "center";
      const labelIndexes = [...new Set([0, Math.floor((values.length - 1) / 2), values.length - 1])];
      labelIndexes.forEach((index) => {
        const time = index === 0
          ? trades[0].opened_at
          : trades[index - 1] && trades[index - 1].closed_at;
        ctx.fillText(shortDate(time), x(index), height - 8);
      });
      ctx.textAlign = "left";
    }
    window.addEventListener("resize", drawChart);
    drawChart();
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
                probability_next_crossing: None,
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
        };
        let data = json!({
            "symbol_key": "TEST",
            "history_requested_1m": 10,
            "history_loaded_1m": 10,
            "history_loaded_1h": 10,
            "trade_summary": report.trade_summary,
            "trades": report.trades,
            "open_trade": report.open_trade,
            "prediction_summary": report.prediction_summary,
            "predictions": report.predictions,
        });
        let html = TEMPLATE.replace("__DATA__", &serde_json::to_string(&data).unwrap());
        assert!(html.contains("<canvas id=\"pnl-chart\"></canvas>"));
        assert!(html.contains("Positions"));
        assert!(html.contains("Entry reason:"));
        assert!(html.contains("Cumulative PnL"));
        assert!(html.contains("Prediction outcomes"));
        assert!(html.contains("Win rate by pattern"));
    }
}
