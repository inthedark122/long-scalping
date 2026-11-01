use std::error::Error;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use super::window_prediction::ResearchReport;

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

pub(crate) fn write_report(path: &Path, report: &ResearchReport) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let (prefix, suffix) = TEMPLATE
        .split_once("__DATA__")
        .ok_or("window prediction template is missing the data placeholder")?;
    let mut output = BufWriter::new(fs::File::create(path)?);
    output.write_all(prefix.as_bytes())?;
    let mut json_writer = HtmlJsonWriter::new(&mut output);
    serde_json::to_writer(&mut json_writer, report)?;
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
  <title>MRC Window Cascade Forecast Chart</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; background: #0b1020; color: #e8eefc; }
    * { box-sizing: border-box; }
    body { margin: 0; background: #0b1020; }
    main { width: min(1500px, 100%); margin: 0 auto; padding: 24px; }
    h1 { margin: 0; font-size: 23px; }
    .sub { color: #91a0bd; margin-top: 6px; font-size: 13px; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(165px, 1fr)); gap: 10px; margin: 20px 0; }
    .card, .panel { background: #131b2f; border: 1px solid #2c3a5a; border-radius: 10px; }
    .card { padding: 13px 15px; }
    .label { color: #91a0bd; font-size: 12px; }
    .value { margin-top: 5px; font-size: 18px; font-weight: 700; overflow-wrap: anywhere; }
    .panel { padding: 16px; margin-top: 14px; }
    .chart-wrap { height: min(68vh, 720px); min-height: 430px; position: relative; overflow-x: auto; overflow-y: hidden; }
    canvas { display: block; width: 100%; height: 100%; }
    #tooltip { display: none; position: absolute; pointer-events: none; z-index: 2; padding: 9px 11px; background: #080d19; border: 1px solid #435273; border-radius: 7px; color: #e8eefc; font-size: 12px; line-height: 1.5; white-space: nowrap; }
    .legend { display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 13px; color: #aab7d1; font-size: 12px; }
    .legend span { display: inline-flex; align-items: center; gap: 6px; }
    .swatch { display: inline-block; width: 15px; height: 3px; border-radius: 3px; }
    .details { color: #aab7d1; font-size: 13px; line-height: 1.6; }
    .details strong { color: #e8eefc; }
    @media (max-width: 650px) { main { padding: 14px; } .chart-wrap { min-height: 360px; } }
  </style>
</head>
<body>
<main>
  <h1>MRC + Movement Cascade · 5m</h1>
  <div class="sub" id="subtitle"></div>
  <section class="cards" id="cards"></section>
  <section class="panel">
    <div class="chart-wrap"><canvas id="chart"></canvas><div id="tooltip"></div></div>
    <div class="legend" id="legend"></div>
  </section>
  <section class="panel details" id="details"></section>
</main>
<script>
const data = __DATA__;
const fmt = (v, digits = 4) => Number.isFinite(v) ? v.toFixed(digits) : 'n/a';
const pct = (v, digits = 2) => Number.isFinite(v) ? `${v >= 0 ? '+' : ''}${v.toFixed(digits)}%` : 'n/a';
const latest = data.latest_forecast;
const lastCandle = data.candles[data.candles.length - 1];
const analysisIndex = Number.isInteger(data.analysis_index) ? data.analysis_index : data.candles.length - 1;
const analysisCandle = data.candles[analysisIndex];
const offsetLabel = data.analysis_offset_minutes % 60 === 0 ? `${data.analysis_offset_minutes / 60}h` : `${data.analysis_offset_minutes}m`;
const durationLabel = minutes => minutes % 60 === 0 ? `${minutes / 60}h` : `${minutes}m`;
const maxProjectionStep = data.latest_projection.reduce((max, bar) => Math.max(max, bar.step), 0);
const overlappingProjectionBars = Math.min(data.realized_future_bars, data.latest_projection.length);
const realBarsAfterTarget = Math.max(data.realized_future_bars - data.latest_projection.length, 0);
const validation = data.validation;
const scrollHint = data.history_loaded > 180 ? ' · scroll horizontally to inspect all candles' : '';
document.getElementById('subtitle').textContent = `${data.symbol_key} · ${data.history_loaded}/${data.history_requested} observed candles · forecast origin ${offsetLabel} before latest · ${data.realized_future_bars} real candles after origin · ${data.latest_projection.length} projected candles · MRC-${data.mrc_period} · ${data.mrc_warmup_bars} warm-up candles · latest closed candle ${lastCandle ? new Date(lastCandle.time_ms + data.timeframe_seconds * 1000).toISOString() : 'no data'}${scrollHint}`;
const cardData = [
  ['Latest real close', lastCandle ? fmt(lastCandle.close) : 'n/a'],
  ['Selected target', latest ? `${fmt(latest.predicted_price)} (${pct(latest.predicted_return_pct)})` : 'no timing estimate'],
  ['Selected duration', latest ? `${durationLabel(latest.duration_minutes)} · ${latest.duration_bars} bars` : 'n/a'],
  ['Target time', latest ? new Date(latest.target_time_ms).toISOString() : 'n/a'],
  ['Real bars since origin', `${data.realized_future_bars} · blue candles`],
  ['Direction confidence', latest ? `${fmt(latest.direction_confidence_pct, 1)}%` : 'n/a'],
  ['Learning samples', latest ? `${latest.training_samples} resolved` : 'building'],
  ['Walk-forward checks', `${validation.resolved_forecasts} · ${validation.direction_accuracy_pct == null ? 'n/a' : fmt(validation.direction_accuracy_pct, 1) + '% direction'}`],
];
document.getElementById('cards').innerHTML = cardData.map(([label, value]) => `<div class="card"><div class="label">${label}</div><div class="value">${value}</div></div>`).join('');
document.getElementById('details').innerHTML = `<strong>Forecast origin:</strong> ${analysisCandle ? new Date(analysisCandle.time_ms + data.timeframe_seconds * 1000).toISOString() : 'unavailable'}, approximately ${offsetLabel} before the latest closed candle. The chart shows real OHLC candles through the latest close. ${overlappingProjectionBars} projected candles overlap real candles after the origin; ${realBarsAfterTarget} additional real candles follow the selected forecast target. Orange candles beyond the latest real data are projections. Every forecast arrow ends at that candle's own target time. <strong>Dynamic timing:</strong> ${data.learning.target_timing} If the cascade has no usable duration estimate, or too few matching outcomes have completed, that candle has no forecast instead of receiving an arbitrary target time. <strong>Learning:</strong> weighted nearest historical analogs use MRC position/slope, recent return, candle body, cascade direction/strength, remaining movement-duration ratio, and the five-movement window. Each historical outcome is used only after its forward interval has closed.<br><strong>Validation:</strong> ${validation.resolved_forecasts} forecasts had a known outcome; mean absolute price error ${validation.mean_absolute_error_pct == null ? 'n/a' : fmt(validation.mean_absolute_error_pct, 3) + '%'} and direction accuracy ${validation.direction_accuracy_pct == null ? 'n/a' : fmt(validation.direction_accuracy_pct, 1) + '%'}. These are walk-forward historical checks, not a guarantee about future performance.`;
const colors = ['#7d8ba8', '#6286ba', '#7da5d5', '#95bce6', '#f2ca66', '#84c7a1', '#6db99c', '#4b9c82', '#347d6a'];
document.getElementById('legend').innerHTML = [
  ...['Down Big', 'Down Outer', 'Down Small', 'Down Inner', 'Mean', 'Up Inner', 'Up Small', 'Up Outer', 'Up Big'].map((name, i) => `<span><i class="swatch" style="background:${colors[i]}"></i>${name}</span>`),
  '<span><i class="swatch" style="background:#6c91ff;height:8px;opacity:.35"></i>5-movement window</span>',
  '<span><i class="swatch" style="background:#55d69b"></i>up forecast arrow</span>',
  '<span><i class="swatch" style="background:#ff7185"></i>down forecast arrow</span>',
  '<span><i class="swatch" style="background:rgba(243,166,74,.45);border:1px dashed #f3a64a;height:8px"></i>projected OHLC candles</span>',
  '<span><i class="swatch" style="background:#54b8ff;height:8px"></i>real candles after forecast origin</span>',
  '<span><i class="swatch" style="background:#54b8ff;border-top:2px dashed #54b8ff"></i>forecast origin</span>',
  '<span><i class="swatch" style="background:#f3a64a;border-top:2px dashed #f3a64a"></i>latest forecast</span>',
].join('');

const canvas = document.getElementById('chart');
const ctx = canvas.getContext('2d');
const tooltip = document.getElementById('tooltip');
const chartWrap = canvas.parentElement;
let geometry = null;
let hoveredIndex = -1;
let initialScrollPositionSet = false;
function drawArrow(fromX, fromY, toX, toY, color, opacity, lineWidth) {
  const angle = Math.atan2(toY - fromY, toX - fromX);
  const head = 6;
  ctx.globalAlpha = opacity;
  ctx.strokeStyle = color; ctx.fillStyle = color; ctx.lineWidth = lineWidth;
  ctx.beginPath(); ctx.moveTo(fromX, fromY); ctx.lineTo(toX, toY); ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(toX, toY);
  ctx.lineTo(toX - head * Math.cos(angle - .48), toY - head * Math.sin(angle - .48));
  ctx.lineTo(toX - head * Math.cos(angle + .48), toY - head * Math.sin(angle + .48));
  ctx.closePath(); ctx.fill();
  ctx.globalAlpha = 1;
}
function draw() {
  const candles = data.candles;
  if (!candles.length) return;
  const margin = { left: 76, right: 22, top: 18, bottom: 35 };
  const rightmostForecastArrow = data.forecasts.reduce(
    (max, forecast, index) => forecast ? Math.max(max, index + forecast.duration_bars) : max,
    candles.length - 1,
  );
  const rightmostProjection = analysisIndex + maxProjectionStep;
  const totalSlots = Math.max(rightmostForecastArrow, rightmostProjection, candles.length - 1) + 2;
  const chartWidth = Math.max(chartWrap.clientWidth, totalSlots * 8 + margin.left + margin.right);
  canvas.style.width = `${chartWidth}px`;
  canvas.style.height = `${chartWrap.clientHeight}px`;
  const ratio = window.devicePixelRatio || 1;
  const bounds = canvas.getBoundingClientRect();
  canvas.width = Math.max(1, Math.round(bounds.width * ratio));
  canvas.height = Math.max(1, Math.round(bounds.height * ratio));
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  const width = bounds.width, height = bounds.height;
  ctx.clearRect(0, 0, width, height);
  const plotW = width - margin.left - margin.right;
  const plotH = height - margin.top - margin.bottom;
  const x = i => margin.left + (i + .5) * plotW / totalSlots;
  const prices = [];
  candles.forEach(c => { prices.push(c.low, c.high, c.mrc.down_big, c.mrc.up_big); if (c.window_low != null) prices.push(c.window_low, c.window_high); });
  data.forecasts.forEach(f => { if (f) prices.push(f.predicted_price); });
  data.latest_projection.forEach(c => prices.push(c.low, c.high));
  if (latest) prices.push(latest.lower_price, latest.upper_price, latest.predicted_price);
  let min = Math.min(...prices), max = Math.max(...prices);
  const padding = Math.max((max - min) * .06, Math.abs(max) * .001);
  min -= padding; max += padding;
  const y = value => margin.top + (max - value) / (max - min) * plotH;
  geometry = { margin, plotW, plotH, x, y, totalSlots, candles };
  const forecastStartX = x(analysisIndex);
  const forecastEndIndex = latest ? analysisIndex + latest.duration_bars : analysisIndex;
  const forecastEndX = x(forecastEndIndex);
  if (latest) {
    ctx.fillStyle = 'rgba(243, 166, 74, .045)';
    ctx.fillRect(forecastStartX, margin.top, forecastEndX - forecastStartX, plotH);
  }
  ctx.font = '11px ui-sans-serif, system-ui';
  ctx.textAlign = 'right'; ctx.textBaseline = 'middle';
  ctx.strokeStyle = '#26334d'; ctx.lineWidth = 1;
  for (let tick = 0; tick <= 5; tick++) {
    const price = max - (max - min) * tick / 5;
    const py = y(price);
    ctx.beginPath(); ctx.moveTo(margin.left, py); ctx.lineTo(width - margin.right, py); ctx.stroke();
    ctx.fillStyle = '#91a0bd'; ctx.fillText(fmt(price, 4), margin.left - 9, py);
  }
  if (candles.some(c => c.window_low != null)) {
    ctx.beginPath();
    let active = false;
    candles.forEach((c, i) => {
      if (c.window_low == null) { active = false; return; }
      if (!active) { ctx.moveTo(x(i), y(c.window_high)); active = true; }
      else ctx.lineTo(x(i), y(c.window_high));
    });
    for (let i = candles.length - 1; i >= 0; i--) {
      const c = candles[i]; if (c.window_low != null) ctx.lineTo(x(i), y(c.window_low));
    }
    ctx.closePath(); ctx.fillStyle = 'rgba(93, 128, 220, .13)'; ctx.fill();
    for (const field of ['window_high', 'window_low']) {
      ctx.beginPath(); let drawing = false;
      candles.forEach((c, i) => {
        if (c[field] == null) { drawing = false; return; }
        if (!drawing) { ctx.moveTo(x(i), y(c[field])); drawing = true; }
        else ctx.lineTo(x(i), y(c[field]));
      });
      ctx.strokeStyle = 'rgba(115, 151, 255, .55)'; ctx.lineWidth = 1; ctx.setLineDash([3, 4]); ctx.stroke(); ctx.setLineDash([]);
    }
  }
  for (let line = 0; line < 9; line++) {
    ctx.beginPath();
    candles.forEach((c, i) => { const value = c.mrc[['down_big','down_outer','down_small','down_inner','mean_line','up_inner','up_small','up_outer','up_big'][line]]; if (i === 0) ctx.moveTo(x(i), y(value)); else ctx.lineTo(x(i), y(value)); });
    ctx.strokeStyle = colors[line]; ctx.lineWidth = line === 4 ? 1.7 : 1; ctx.globalAlpha = line === 4 ? .95 : .78; ctx.stroke(); ctx.globalAlpha = 1;
  }
  data.forecasts.forEach((forecast, index) => {
    if (!forecast) return;
    const source = candles[index];
    const selected = index === analysisIndex;
    const color = selected ? '#f3a64a' : (forecast.predicted_return_pct >= 0 ? '#55d69b' : '#ff7185');
    const highlighted = selected || index === hoveredIndex;
    drawArrow(
      x(index), y(source.close),
      x(index + forecast.duration_bars), y(forecast.predicted_price),
      color, highlighted ? .95 : .32, highlighted ? 2.4 : 1.05,
    );
  });
  const candleW = Math.max(2, Math.min(10, plotW / totalSlots * .58));
  const projectedW = Math.max(2, candleW * .78);
  data.latest_projection.forEach(bar => {
    const cx = x(analysisIndex + bar.step);
    ctx.globalAlpha = .88;
    ctx.strokeStyle = '#f3a64a'; ctx.fillStyle = 'rgba(243,166,74,.22)'; ctx.lineWidth = 1.2;
    ctx.beginPath(); ctx.moveTo(cx, y(bar.high)); ctx.lineTo(cx, y(bar.low)); ctx.stroke();
    const bodyTop = Math.min(y(bar.open), y(bar.close));
    const bodyHeight = Math.max(1, Math.abs(y(bar.open) - y(bar.close)));
    ctx.fillRect(cx - projectedW / 2, bodyTop, projectedW, bodyHeight);
    ctx.setLineDash([2, 2]); ctx.strokeRect(cx - projectedW / 2, bodyTop, projectedW, bodyHeight); ctx.setLineDash([]);
    ctx.globalAlpha = 1;
  });
  candles.forEach((c, i) => {
    const cx = x(i), up = c.close >= c.open;
    const afterOrigin = i > analysisIndex;
    const color = afterOrigin ? '#54b8ff' : (up ? '#55d69b' : '#ff7185');
    const bodyW = afterOrigin ? candleW * .5 : candleW;
    ctx.strokeStyle = color; ctx.fillStyle = color; ctx.lineWidth = afterOrigin ? 1.5 : 1.2;
    ctx.beginPath(); ctx.moveTo(cx, y(c.high)); ctx.lineTo(cx, y(c.low)); ctx.stroke();
    const bodyTop = Math.min(y(c.open), y(c.close));
    const bodyHeight = Math.max(1, Math.abs(y(c.open) - y(c.close)));
    ctx.fillRect(cx - bodyW / 2, bodyTop, bodyW, bodyHeight);
  });
  ctx.strokeStyle = 'rgba(84,184,255,.78)'; ctx.lineWidth = 1; ctx.setLineDash([4, 5]);
  ctx.beginPath(); ctx.moveTo(forecastStartX, margin.top); ctx.lineTo(forecastStartX, height - margin.bottom); ctx.stroke(); ctx.setLineDash([]);
  ctx.textAlign = 'center'; ctx.textBaseline = 'top'; ctx.fillStyle = '#8ed5ff';
  ctx.fillText(`origin -${offsetLabel}`, forecastStartX, margin.top + 3);
  const latestX = x(candles.length - 1);
  if (latestX !== forecastStartX) {
    ctx.strokeStyle = 'rgba(84,184,255,.35)'; ctx.setLineDash([2, 5]);
    ctx.beginPath(); ctx.moveTo(latestX, margin.top); ctx.lineTo(latestX, height - margin.bottom); ctx.stroke(); ctx.setLineDash([]);
  }
  if (latest) {
    ctx.strokeStyle = 'rgba(243, 166, 74, .65)'; ctx.lineWidth = 1; ctx.setLineDash([4, 5]);
    ctx.beginPath(); ctx.moveTo(forecastEndX, margin.top); ctx.lineTo(forecastEndX, height - margin.bottom); ctx.stroke(); ctx.setLineDash([]);
  }
  if (latest) {
    const toX = forecastEndX;
    ctx.fillStyle = 'rgba(243,166,74,.18)';
    const bandTop = y(Math.max(latest.lower_price, latest.upper_price)), bandBottom = y(Math.min(latest.lower_price, latest.upper_price));
    ctx.fillRect(toX - 7, bandTop, 14, bandBottom - bandTop);
    ctx.strokeStyle = '#f3a64a'; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(toX, bandTop); ctx.lineTo(toX, bandBottom); ctx.stroke();
    ctx.fillStyle = '#f3a64a'; ctx.beginPath(); ctx.arc(toX, y(latest.predicted_price), 4, 0, Math.PI * 2); ctx.fill();
    ctx.textAlign = 'center'; ctx.textBaseline = 'bottom'; ctx.fillStyle = '#ffd29d'; ctx.fillText(`forecast ${fmt(latest.predicted_price)}`, toX, y(latest.predicted_price) - 7);
  }
  ctx.fillStyle = '#91a0bd'; ctx.textAlign = 'center'; ctx.textBaseline = 'top';
  const labelStep = Math.max(1, Math.floor(candles.length / 8));
  for (let i = 0; i < candles.length; i += labelStep) ctx.fillText(new Date(candles[i].time_ms).toISOString().slice(5, 16).replace('T', ' '), x(i), height - margin.bottom + 9);
  for (let step = 12; step < maxProjectionStep; step += 12) {
    ctx.fillText(`+${step * data.timeframe_seconds / 60}m`, x(analysisIndex + step), height - margin.bottom + 9);
  }
  if (latest) {
    ctx.fillStyle = '#ffd29d'; ctx.fillText(`+${durationLabel(latest.duration_minutes)} target`, forecastEndX, height - margin.bottom + 9);
  }
  if (!initialScrollPositionSet) {
    chartWrap.scrollLeft = chartWrap.scrollWidth;
    initialScrollPositionSet = true;
  }
}
canvas.addEventListener('pointermove', event => {
  if (!geometry) return;
  const rect = canvas.getBoundingClientRect();
  const viewportRect = chartWrap.getBoundingClientRect();
  const index = Math.round((event.clientX - rect.left - geometry.margin.left) / geometry.plotW * geometry.totalSlots - .5);
  const c = geometry.candles[index];
  if (!c) {
    if (hoveredIndex !== -1) { hoveredIndex = -1; draw(); }
    const step = index - analysisIndex;
    const projected = data.latest_projection.find(bar => bar.step === step);
    if (!projected) { tooltip.style.display = 'none'; return; }
    tooltip.innerHTML = `<strong>Projected candle +${step} (${new Date(projected.time_ms).toISOString()})</strong><br>O ${fmt(projected.open)} · H ${fmt(projected.high)} · L ${fmt(projected.low)} · C ${fmt(projected.close)}<br>Weighted analog projection · selected target ${latest ? fmt(latest.predicted_price) + ' at ' + new Date(latest.target_time_ms).toISOString() : 'n/a'}`;
    tooltip.style.display = 'block';
    const left = Math.min(event.clientX - viewportRect.left + 14, viewportRect.width - 300);
    const top = Math.max(8, event.clientY - viewportRect.top - 24);
    tooltip.style.left = `${left}px`; tooltip.style.top = `${top}px`;
    return;
  }
  if (hoveredIndex !== index) { hoveredIndex = index; draw(); }
  const f = data.forecasts[index];
  const targetTime = f ? new Date(f.target_time_ms).toISOString() : null;
  const realizedStep = index - analysisIndex;
  const projected = realizedStep > 0 ? data.latest_projection.find(bar => bar.step === realizedStep) : null;
  const comparison = projected
    ? `<br><strong>Origin forecast comparison (+${realizedStep}):</strong> predicted C ${fmt(projected.close)} · real C ${fmt(c.close)} · close error ${pct((c.close / projected.close - 1) * 100)}`
    : '';
  tooltip.innerHTML = `<strong>${new Date(c.time_ms).toISOString()}${index > analysisIndex ? ' · REAL' : ''}</strong><br>O ${fmt(c.open)} · H ${fmt(c.high)} · L ${fmt(c.low)} · C ${fmt(c.close)}<br>MRC position ${fmt((c.close - c.mrc.mean_line) / Math.max(c.mrc.up_big - c.mrc.down_big, Number.EPSILON), 3)} · slope ${fmt(c.mrc.mean_line_slope, 6)}<br>Cascade ${c.cascade_phase} · dir ${c.cascade_direction} · strength ${fmt(c.cascade_strength, 2)}<br>Window ${c.window_low == null ? 'warming up' : `${fmt(c.window_low)}–${fmt(c.window_high)} · ${c.window_direction_changes} turns · ${c.window_price_location}`}<br>${f ? `${durationLabel(f.duration_minutes)} target ${fmt(f.predicted_price)} (${pct(f.predicted_return_pct)}) at ${targetTime} · ${f.direction_confidence_pct.toFixed(1)}% confidence · ${f.training_samples} samples` : 'No forecast: duration or analog history unavailable'}${comparison}`;
  tooltip.style.display = 'block';
  const left = Math.min(event.clientX - viewportRect.left + 14, viewportRect.width - 300);
  const top = Math.max(8, event.clientY - viewportRect.top - 24);
  tooltip.style.left = `${left}px`; tooltip.style.top = `${top}px`;
});
canvas.addEventListener('pointerleave', () => { tooltip.style.display = 'none'; if (hoveredIndex !== -1) { hoveredIndex = -1; draw(); } });
window.addEventListener('resize', draw);
draw();
</script>
</body>
</html>"###;
