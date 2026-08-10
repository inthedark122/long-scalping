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
        "latest_event_time_ms": report.latest_event.map(|event| event.time_ms),
        "trade_summary": report.trade_summary,
        "trades": report.trades,
        "open_trade": report.open_trade,
        "wait_mode": report.wait_mode,
        "wait_direction": report.wait_direction,
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
    .positive { color: var(--green); }
    .negative { color: var(--red); }
    .open { color: var(--blue); }
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
              <th>MFE</th><th>MAE</th><th>Duration</th>
            </tr>
          </thead>
          <tbody id="positions"></tbody>
        </table>
      </div>
    </section>
  </main>

  <script>
    const DATA = __DATA__;
    const trades = DATA.trades || [];
    const summary = DATA.trade_summary || {};
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

    document.getElementById("context").textContent =
      DATA.symbol_key + " · 1m history " + DATA.history_loaded_1m +
      " · generated from " + DATA.history_loaded_1h + " hourly candles" +
      (DATA.latest_event_time_ms ? " · as of " + date(DATA.latest_event_time_ms) : "");

    const cardData = [
      ["Trades", summary.trades || 0],
      ["Win rate", Number(summary.win_rate_pct || 0).toFixed(1) + "%"],
      ["Gross PnL", Number(summary.total_gross_pnl_pct || 0).toFixed(4) + "%"],
      ["Fees", Number(summary.total_fees_pct || 0).toFixed(4) + "%"],
      ["Net PnL", Number(summary.total_pnl_pct || 0).toFixed(4) + "%"],
      ["Max drawdown", Number(summary.max_drawdown_pct || 0).toFixed(4) + "%"],
      ["Average PnL", Number(summary.average_pnl_pct || 0).toFixed(4) + "%"],
      ["Average duration", summary.average_duration_ms
        ? (Number(summary.average_duration_ms) / 60000).toFixed(2) + "m" : "—"],
      ["Wait mode", DATA.wait_mode ? "ON" : "OFF"],
    ];
    document.getElementById("cards").innerHTML = cardData.map(([label, value]) =>
      '<div class="card"><div class="card-label">' + esc(label) +
      '</div><div class="card-value">' + esc(value) + '</div></div>'
    ).join("");

    const rows = trades.map((trade, index) => {
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
        '<td class="positive">' + money(trade.mfe_pct) + '</td>' +
        '<td class="negative">' + money(-Math.abs(Number(trade.mae_pct || 0))) + '</td>' +
        '<td>' + duration(trade.opened_at, trade.closed_at) + '</td>' +
        '</tr>';
    });
    if (DATA.open_trade) {
      const trade = DATA.open_trade;
      const side = trade.side > 0 ? "LONG" : "SHORT";
      rows.push(
        '<tr class="open"><td>open</td><td>' + side + '</td><td>' +
        esc(side + " " + trade.source_name + " → " + trade.target_name) +
        '</td><td>' + esc(date(trade.opened_at)) + '</td><td>' +
        price(trade.entry_price) + '</td><td>' + price(trade.target_price) +
        '</td><td>' + price(trade.stop_price) +
        '</td><td>—</td><td>—</td><td>OPEN</td><td>—</td><td>—</td><td>—</td><td>—</td><td>—</td><td>—</td></tr>'
      );
    }
    document.getElementById("positions").innerHTML = rows.join("") ||
      '<tr><td colspan="16" class="subtle">No positions</td></tr>';

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
    use crate::research::mrc_crossing::TradeSummary;

    #[test]
    fn generated_report_is_standalone_html() {
        let report = AnalysisReport {
            crossing_count: 0,
            transition_count: 0,
            break_event_count: 0,
            scenario_event_count: 0,
            transitions: Vec::new(),
            scenario_transitions: Vec::new(),
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
            trade_summary: TradeSummary::default(),
            trades: Vec::new(),
            open_trade: None,
            wait_mode: false,
            wait_direction: 0,
        };
        let data = json!({
            "symbol_key": "TEST",
            "history_requested_1m": 10,
            "history_loaded_1m": 10,
            "history_loaded_1h": 10,
            "trade_summary": report.trade_summary,
            "trades": report.trades,
        });
        let html = TEMPLATE.replace("__DATA__", &serde_json::to_string(&data).unwrap());
        assert!(html.contains("<canvas id=\"pnl-chart\"></canvas>"));
        assert!(html.contains("Positions"));
        assert!(html.contains("Cumulative PnL"));
    }
}
