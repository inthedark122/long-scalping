# MRC movement pattern-match solution

**Purpose:** a research view of MRC and price movement from real OHLC klines. It describes evidence and a measurement design; it does not change trading behavior or claim a profitable signal.

## What the current pipeline measures

The module calculates MRC-200 separately for 1m and 5m klines. The channel uses a two-pole smoother on typical price and true range. Its nine lines are the smoothed middle plus or minus scaled smoothed range. Each line moves as new bars arrive, so a line is a changing boundary rather than a fixed price level. See the [MRC implementation](../module/ta-rs/src/indicators/mean_reversion_channel.rs) and [market data snapshot](../module/src/market_data.rs).

Realtime movement is assembled from several distinct sources:

1. The 1m MRC lines and live price feed line touches and crossings into the [V-line tracker](../module/src/detection/vlines.rs).
2. The [MRC crossing model](../module/src/detection/mrc_crossing.rs) learns routes between line indices and evaluates every matching local pattern. Pattern identity and trade selection are separate stages.
3. The [touch state](../module/src/detection/mrc_touch.rs) tracks short-decay price speed, proximity, side changes, MRC width, and width change. The [touch learner](../module/src/detection/mrc_touch_learning.rs) records the next line after an exact touch in a 5m slope/price-side context.
4. The [Movement Cascade](../module/src/detection/cascade.rs) describes direction, phase, strength, speed, potential movement, adverse movement, and recent movement duration.
5. The [Movement Window](../module/src/detection/movement_window.rs) uses the last five completed cascade movements to find a local high/low and count direction changes. That range is separate from the MRC lower/upper bands.

The existing pattern vocabulary is documented in [MRC movement pattern classification](mrc-pattern-classification.md): `STAIRS_UP`, `STAIRS_DOWN`, `FAST_BREAK_RETURN`, `RANGE_CYCLE_SIDEWAY`, and `TRENDING_CYCLE`. These are local structural descriptions. A pattern label alone is not a calibrated forecast.

| Pattern | Direction read | Zone/movement read | Future question to measure |
| --- | --- | --- | --- |
| `STAIRS_UP` | Upward continuation while protected swing structure holds | Higher MRC lines reached with intervening pullbacks | Does the next unbroken upper line get reached before a close breaks the protected swing? |
| `STAIRS_DOWN` | Downward continuation while protected swing structure holds | Lower MRC lines reached with intervening pullbacks | Does the next unbroken lower line get reached before a close breaks the protected swing? |
| `FAST_BREAK_RETURN` | No direction from the return alone | Crossed line is a candidate barrier | After return to the source line, does price reject the crossed line or reclaim it? |
| `RANGE_CYCLE_SIDEWAY` | No strong net direction | Repeated lower/upper visits with limited net drift | Does the opposite edge get reached before the local range breaks? |
| `TRENDING_CYCLE` | Direction follows net drift | Repeated zone crossings inside a moving range | Does the move continue with drift or resolve as a pullback? |

### Important current behavior

- `MrcSignal.pzone` normalizes price between Down Big and Up Big, then clamps values to 0–1. `MrcSignal.zone` counts lines at or below price and subtracts four. It is a stepped line index, not a symmetric zone probability; price exactly at Mean is counted above Mean. A chart block should show the actual line interval and retain an outside-channel flag.
- `MrcTouchSignal` provides continuous speed and width-change measurements, but those are short-lived context features. Its EMA time constants are about 90 seconds for the general signal and 45 seconds for crossing debounce. It does not retain a four-hour line-hold state.
- `MrcDirectionPrediction` currently emits a one-hot up/down result only when touch learning is reliable; otherwise it emits one-hot sideways. Those values are routing/gating state, not calibrated probabilities. The live direction view should not display them as forecast odds.
- The live 5m MRC slope and structure are context for a 1m candidate. The Movement Window is a separate recent price range and has its own sideways strategy. Keep both visible so an MRC zone is not mistaken for a cascade range edge.

Live startup requests 680 one-minute bars, using the first 200 to warm MRC-200 and replaying the following 480 bars; it requests 200 five-minute bars to warm five-minute MRC-200. The window research report uses 400 earlier 5m bars to warm the recursive MRC and cascade state. MRC-200 values can therefore differ near a replay's start. This study uses only values after the stated warm-up; future comparisons should keep this initialization difference explicit.

## Research questions and candidate measurements

Keep the five questions separate; do not compress direction, movement size, zone, forecast, and line persistence into one score. The table is the broader research schema, not a claim that every field is already emitted by the CAP gallery below. That gallery currently compares same-class historical examples and displays the available outcomes; it does not yet include normalized slope/cascade rows, raw speed, or Movement Window location.

| Question | Research field | Display |
| --- | --- | --- |
| Possible movement direction | Normalized MRC mean slope, cascade direction/phase, active staircase direction, and whether those agree | Up, down, neutral, or conflict. Keep the contributing signals visible. |
| Movement increasing or decreasing | Signed price speed and its absolute magnitude; change in MRC band width percent over a named interval | Direction and movement size in separate rows. Band expansion means volatility is increasing; it does not mean price is rising. |
| Lower or upper trading zone | Explicit MRC line interval, continuous position between Down Big and Up Big, distance to nearest line, and Movement Window location | Lower, mean, upper, outside-channel, and local-range position as distinct measures. |
| Possible future movement | Walk-forward outcome distribution to a specified horizon or next MRC line, with resolved count and outcome range | Up/down/flat estimates with horizon and sample count. Show unknown when evidence is insufficient. |
| MRC line non-break over 4+ hours | Touch line, approach side, wick/close behavior, time to first close breach, line movement, and surviving duration | Per-line hold age and test count. Separate a cross of the recalculated line from a cross of the line price captured at the touch. |

For cross-timeframe comparisons, normalize slope by current MRC width and state the interval. A raw 1m slope cannot be compared directly with a raw 5m slope. Keep the line identity and timeframe in every event.

For the zone view, retain the exact adjacent-line interval and give each close a coarse lower, mean-vicinity, upper, or outside-channel class. Preserve raw channel position and explicit above-Up-Big/below-Down-Big flags so channel saturation does not hide an outside move. Show Movement Window position as a separate contextual field; it is not an MRC boundary.

## Real-kline pattern match sample

Open the [five-chart HTML gallery](../tmp/reports/mrc-pattern-match-CAP-1m-20261101.html) or its [match and measurement JSON](../tmp/reports/mrc-pattern-match-CAP-1m-20261101.json). The gallery compares two actual price histories in every chart: the latest classified event on the left and its best earlier match on the right. Each panel draws the report's observed one-minute OHLC candles and locally calculated MRC-200 lines at their actual prices. The colored blocks are inside those price charts; there is no separate heatmap pane, second chart, or generated price candle.

![CAP actual-kline direction comparison](../tmp/reports/mrc-pattern-match-CAP-1m-20261101-01-direction.svg)

The sample classifier event is `STAIRS_UP`, direction `+1`, lifecycle `pullback`, at **2026-11-01 06:31 UTC**. Its class-specific MRC path is `[2, 3, 4, 5, 4, 5, 4, 3]`, using the line order above. The best earlier match is **2026-10-29 19:46 UTC**, with normalized line-sequence DTW distance **0.03125**. It is selected from five earlier `STAIRS_UP` examples with the same direction. The match ranking compares class-specific MRC line-index sequences; it does not compare unrelated indicator values or pick a visually similar chart by eye.

The five facets answer separate questions:

1. [Direction](../tmp/reports/mrc-pattern-match-CAP-1m-20261101-01-direction.svg): the top five matches voted up **3**, down **2**, flat **0**. This is a small historical vote, not a 60% probability.
2. [Movement pace](../tmp/reports/mrc-pattern-match-CAP-1m-20261101-02-movement-pace.svg): compare the four-hour realized range with the range of the classified pattern. The median ratio was **1.47×** across the five matches; the best match itself reached **2.26×**. A larger range says movement expanded, not that it rose.
3. [Lower or upper zone](../tmp/reports/mrc-pattern-match-CAP-1m-20261101-03-lower-upper-zone.svg): blocks follow the exact adjacent MRC-line interval containing each actual close. The labels retain the actual line indices and names, rather than collapsing the channel into an inferred probability zone.
4. [Four-hour movement](../tmp/reports/mrc-pattern-match-CAP-1m-20261101-04-future-movement.svg): prior resolved outcomes had a median return of **+2.142%**, with a 10th–90th percentile range of **−2.417% to +4.418%**. The band is a historical analog summary drawn from the query origin; it is not a predicted price path. The query's own four-hour close outcome is right-censored because only about **75 minutes** remain in the loaded history after the event.
5. [Four-hour MRC line test](../tmp/reports/mrc-pattern-match-CAP-1m-20261101-05-four-hour-line-tests.svg): one of the five historical matches held its tested moving line for four hours, two broke, and two had no qualifying touch. The query's protected-line test broke on the touch candle (**0 minutes**). This is a separate price-vs-moving-line test: the classifier still labels the structural pattern lifecycle `pullback`. Those labels answer different questions and should not be merged.

The best match's actual 4h close return was **+2.142%**. Its complete match outcomes include both up and down results, so the positive median is not evidence of a dependable forecast. Candidate outcomes are required to finish before the query pattern begins; the query's later bars are never used as match outcomes. See the JSON's `historical_match_classification.method` for the matching and tie-break rules. Chart 4's dashed band is the only forward-looking annotation in the five charts; all candle bodies, wicks, and MRC lines are observed values.

The source report spans **2026-10-25 09:07 UTC through 2026-11-01 07:46 UTC** and contains the requested and loaded **10,000** contiguous one-minute klines. The [source crossing JSON](../tmp/reports/mrc-crossing-OKX_CAP-USDT-SWAP_SWAP.json) and [source HTML report](../tmp/reports/mrc-crossing-OKX_CAP-USDT-SWAP_SWAP.html) include all bars and crossing analysis. Rebuild the five overlays with the [pattern match chart generator](../scripts/build_mrc_pattern_match_charts.py).

## Earlier independent five-minute line-persistence baseline

This separate APR baseline uses 5m candles. MRC lines move, so “held” means **price stayed on the same side of the recalculated line** after a touch. An episode starts when a candle wick intersects a line after a preceding non-touch candle. The previous close gives the approach side. A close on the opposite side of the moving line is a break. A four-hour test requires 48 subsequent observed 5m bars; unfinished tests at the end of the source history are censored. It is not the CAP one-minute pattern-matching sample above.

This definition measures relative-side persistence. It does not prove that price respected a fixed support/resistance price: a moving MRC line can move through price. A fuller knowledge base should record both the moving-line side change and whether price later crosses the line price captured at the touch. The two measures answer different questions.

## Real kline evidence

### Four-hour line persistence: APR, 5m

Source: 864 observed APR five-minute candles from **2026-10-29 07:10 UTC through 2026-11-01 07:05 UTC**. MRC-200 is calculated from the actual OHLC bars with 400 earlier bars used for warm-up. Source candles, timestamps, OHLC bounds, MRC ordering, and 5m spacing passed the local report checks.

There were 121 touch episodes. Three ended too close to the data boundary for a complete four-hour outcome and are excluded from the hold-rate denominator. **Thirteen of 118 complete episodes (11.0%)** stayed on the approach side of the recalculated line for four hours. This is a short, symbol-specific sample with overlapping outcome windows, so the rate is descriptive and not a general line reliability estimate.

| MRC line | Complete tests | Held 4h | Rate |
| --- | ---: | ---: | ---: |
| Down Big | 0 | 0 | n/a |
| Down Outer | 1 | 1 | 100% |
| Down Small | 3 | 1 | 33.3% |
| Down Inner | 18 | 2 | 11.1% |
| Mean | 46 | 0 | 0% |
| Up Inner | 30 | 4 | 13.3% |
| Up Small | 13 | 2 | 15.4% |
| Up Outer | 5 | 3 | 60.0% |
| Up Big | 2 | 0 | 0% |

Outer-line estimates have small denominators (zero to five complete tests). The mean-line sample did not produce a four-hour same-side hold under this definition. This supports tracking line-specific counts and duration rather than coloring a line “strong” from its name alone.

Two actual-only chart samples show the measurement:

- [APR Up Small resistance test held for four hours](../tmp/reports/mrc-line-persistence-APR-5m-20261101-hold.svg): touch at **2026-10-29 16:35 UTC**, MRC line about **0.0632149**, touch candle close **0.06260**. No later close crossed to the other side of the recalculated line during the 48-bar window.
- [APR Mean support test broke on the touch candle](../tmp/reports/mrc-line-persistence-APR-5m-20261101-break.svg): touch at **2026-10-29 10:10 UTC**, MRC Mean about **0.0508431**, candle close **0.04984**. The touch candle closed below the line, so it failed the support test immediately.

Both charts draw observed OHLC candles and MRC-200 lines only. They contain no projected price candles. The accompanying [line-persistence HTML report](../tmp/reports/mrc-line-persistence-APR-5m-20261101.html) and [JSON measurements](../tmp/reports/mrc-line-persistence-APR-5m-20261101.json) contain the method and counts.

### Crossing-replay context

The 1m crossing replay gives broader structural context across CAP, BEAT, and ETH. Each report contains **10,000 requested and loaded klines**, refreshed from the project market-data API; the endpoints differ by one minute. It shows classifier crossing/prediction counts and virtual trade accounting. Route-following counts are not directional accuracy, and trade results do not measure the quality of the class-sequence match above.

| Symbol | 1m bars | Crossings | Predictions (filled) | Followed / broken / unresolved | Entries = completed | Wins / losses | Win rate | Gross / fees / net PnL | Max DD | Active / pending / open |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| CAP | 10,000 | 461 | 254 (229) | 59 / 190 / 5 | 62 | 12 / 50 | 19.4% | −42.915% / 4.006% / −46.922% | 48.665% | 0 / 0 / 0 |
| BEAT | 10,000 | 573 | 483 (423) | 130 / 352 / 1 | 59 | 19 / 40 | 32.2% | +46.400% / 3.560% / +42.840% | 13.959% | 0 / 0 / 0 |
| ETH | 10,000 | 665 | 965 (935) | 297 / 657 / 11 | 61 | 16 / 45 | 26.2% | +2.532% / 3.820% / −1.288% | 5.354% | 0 / 0 / 0 |

These are virtual trade outputs, not evidence that a movement pattern is profitable or that a direction forecast is correct. The report's `followed` count is route-following behavior; it is not trade wins. Open the [CAP](../tmp/reports/mrc-crossing-OKX_CAP-USDT-SWAP_SWAP.html), [BEAT](../tmp/reports/mrc-crossing-OKX_BEAT-USDT-SWAP_SWAP.html), and [ETH](../tmp/reports/mrc-crossing-OKX_ETH-USDT-SWAP_SWAP.html) HTML reports; each has a matching JSON report in `tmp/reports/`.

### Data quality

The refreshed report payloads each contain the requested 10,000 visible candles; all have valid OHLC bounds, ordered positive MRC lines, and no one-minute gaps. The pattern-chart generator validates these conditions again before drawing. This is a data-shape check, not a substitute for source-exchange reconciliation. NTPS was unavailable and treated as zero in the crossing reports.

## Pattern-solution rules for future research

1. Keep `pattern_kind` as structure and keep direction, movement size, zone, forecast, and line persistence as separate measurements.
2. Report a forecast with its horizon, target line, matched sample count, outcome range, and resolved date range. Use chronological walk-forward evaluation; do not train on any candle after the prediction origin.
3. Calibrate direction as up/down/flat from later observed klines. Until calibration and a larger independent sample exist, show the current direction as `uncertain` when evidence conflicts or is missing.
4. Record wick contact, close acceptance, current moving-line value, and touch-time line value separately. Count test episodes and censored observations so a percentage always has a denominator.
5. Show MRC lower/mean/upper location alongside Movement Window location. Their boundaries come from different calculations.
6. Use real closed klines for every chart. Any forecast or analog projection must be visually marked as model output and excluded from examples of observed price behavior.

This design is an analysis knowledge base. It does not grant an entry, exit, or risk-management decision.
