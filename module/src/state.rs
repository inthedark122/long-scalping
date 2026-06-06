use lte_strategy_bridge::indicator_access::IndicatorHistory;
use std::collections::{HashMap, HashSet};

// ── Per-timeframe movement state ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TfMovState {
    pub candles: u64,
    pub cross: i8,         // current zone (-4..+4), 0 = MeanLine zone
    pub history: [i8; 20], // ring buffer of cross levels
    pub hist_pos: usize,   // next write index
    pub hist_len: usize,   // valid entries (0..=20)
    pub floor: i8,         // min(history) — deepest pullback seen
    pub ceiling: i8,       // max(history) — highest push seen
    pub floor_age: u64,    // candles since floor last changed
    pub ceiling_age: u64,
    pub angle_sign: i8, // +1 / -1 / 0
    pub angle_sign_candles: u64,
    pub mov: f64,       // slow movement score -100..+100
    pub regime: i8,     // 0=UNK  1=Sideways  2=TrendUP  -2=TrendDOWN
    pub conf: u8,       // confidence 0-100
    pub cross_prev: i8, // cross level from the previous candle
}

impl Default for TfMovState {
    fn default() -> Self {
        Self {
            candles: 0,
            cross: 0,
            history: [0i8; 20],
            hist_pos: 0,
            hist_len: 0,
            floor: 0,
            ceiling: 0,
            floor_age: 0,
            ceiling_age: 0,
            angle_sign: 0,
            angle_sign_candles: 0,
            mov: 0.0,
            regime: 0,
            conf: 0,
            cross_prev: 0,
        }
    }
}

// ── V-movement tracker ──────────────────────────────────────────────────────
//
// One VLineTracker per MRC line (9 total). Each line tracks its own V history
// independently — no migration or reset when price moves between lines.
// Long side: Vs where price crossed above that line and returned.
// Short side: Vs where price crossed below that line and returned.

#[derive(Debug, Clone, Copy, Default)]
pub struct VSide {
    pub count: u8,       // closed Vs recorded on this side
    pub last_power: f64, // penetration % of most recent closed V
    pub prev_power: f64, // penetration % of V before that
    pub exhausted: bool, // last_power < prev_power * 0.70
    pub stair: bool,     // last_power > prev_power * 1.10 (momentum building)
}

#[derive(Debug, Clone, Copy)]
pub struct VLineTracker {
    pub was_above: Option<bool>, // previous tick: was price >= this line? (None = uninit)
    pub peak: Option<f64>,       // max price seen while above this line (open long V)
    pub trough: Option<f64>,     // min price seen while below this line (open short V)
    pub long_side: VSide,        // Vs that crossed above this line
    pub short_side: VSide,       // Vs that crossed below this line
    // Real-time crossing recency (sug ticks) — feeds organic entry freshness (Timing 0.2).
    // Updated every time price crosses this line on a SugInfo tick, not just on 1m closes.
    pub last_cross_tick: u64, // state.ticks at last real-time crossing (0 = never)
    pub last_cross_dir: i8,   // +1 = crossed up (support test), -1 = crossed down (resistance test)
}

impl Default for VLineTracker {
    fn default() -> Self {
        Self {
            was_above: None,
            peak: None,
            trough: None,
            long_side: VSide::default(),
            short_side: VSide::default(),
            last_cross_tick: 0,
            last_cross_dir: 0,
        }
    }
}

// ── Fast candle tracker ─────────────────────────────────────────────────────
//
// Groups consecutive high-volume directional candles into one "event":
//   - event ends when 2+ calm candles pass in a row
//   - rolling EMA stats built up in real-time (no historical data needed)
//
// Pre-event signal: elevated volume with ambiguous BSG often precedes a
// directional spike — fire before the actual fast candle starts.
#[derive(Debug, Clone)]
pub struct FastCandleStats {
    // In-progress event
    pub active: bool,
    pub dir: i8, // +1 bull, -1 bear
    pub start_price: f64,
    pub start_candle: u64,
    pub peak_gain: f64,  // max Gain seen
    pub peak_bsg: f64,   // max |BSG| seen
    pub pause: u8,       // consecutive non-fast candles mid-event (end at > 2)
    pub calm_since: u64, // candle when last event ended (0 = init)

    // EMA rolling statistics (α = 0.20, initialised from first event)
    pub avg_pct: f64,  // avg event price-move in %
    pub avg_gain: f64, // avg peak Gain per event
    pub avg_dur: f64,  // avg duration in 1m candles
    pub avg_calm: f64, // avg calm candles between events
    pub count: u32,    // total completed events

    // Pre-event signal: high-volume near-neutral BSG → often precedes spike
    pub prefire: i8,         // +1 bull prefire, -1 bear prefire, 0 = none
    pub prefire_candle: u64, // expires after 3 candles of inactivity

    // Momentum: decaying directional sum (each event adds ±1.0, decays × 0.85)
    pub momentum: f64, // > 0 = recent bull bias, < 0 = bear bias

    // Sliding window of last 8 completed events (oldest first, newest last)
    pub history: Vec<FcEvent>,

    // Direction + tick of the last COMPLETED event (drives the fast-candle scalp).
    pub last_end_dir: i8,
    pub last_end_tick: u64,
}

impl Default for FastCandleStats {
    fn default() -> Self {
        Self {
            active: false,
            dir: 0,
            start_price: 0.0,
            start_candle: 0,
            peak_gain: 0.0,
            peak_bsg: 0.0,
            pause: 0,
            calm_since: 0,
            avg_pct: 0.0,
            avg_gain: 0.0,
            avg_dur: 0.0,
            avg_calm: 0.0,
            count: 0,
            prefire: 0,
            prefire_candle: 0,
            momentum: 0.0,
            history: Vec::new(),
            last_end_dir: 0,
            last_end_tick: 0,
        }
    }
}

// ── Fast candle history entry ─────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct FcEvent {
    pub dir: i8,  // +1 bull, -1 bear
    pub pct: f64, // price move %
    pub dur: u64, // duration in ticks
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Config {
    pub breakout_guard: bool,
    pub imbalance_guard: bool,
    pub margin_sl: bool,
    pub imbalance_ratio: f64,
    pub breakout_ntps: i64,
    pub sl_margin_pct: f64,
    pub seed: f64,
    pub cap: f64,
    pub cooldown_ticks: u64,
    pub nearest: usize,
    // v0.55 (0.A): risk-based sizing — size by risk, not arbitrary multipliers, capped at max_amount.
    pub risk_pct: f64, // risk budget = this fraction of allocated margin (max_amount/leverage)
    pub stop_pct: f64, // stop distance as fraction of price; loss@stop = notional × stop_pct
    // v0.57: runtime debug-info builder. Selects FULL-detail topic blocks appended to EVERY debug
    // line (init/ind/sug) so the API snapshot — whichever event it lands on — always carries the
    // view we want, readable straight from `list_bots` last_debug with no extra indicator/tool
    // calls. 0 = default per-event logs only. Change any time via bot-group config — no redeploy.
    // Bitmask preset (add to combine):
    //   1 V   — V-tracker table (per-MRC-line resistance/support V counts, power%, exhausted)
    //   2 MRC — 1m band price map (DB..UB) + 5m mean-line
    //   4 BIG — Big-line bracket state: long big_tested flag
    //   8 CAS — movement cascade / run leg / stair-cross / 5m hard-break
    //  16 DMP — uncertainty damp + risk drawdown
    //  31 ALL — everything
    pub debug_info: i64,
    // Entry mode switches (v0.29.0)
    pub enable_stairs: bool, // candle-close stair opens, kickstart, trend-DCA (dca-l/s)
    pub enable_trend: bool, // zone-rejection + fast-candle breakdown entries, zone-SL
    // Fast-candle scalp (opt-in experiment): buy the flush low after a fast-DOWN event,
    // take profit on a small bounce / fast-up event, hard stop at DownBig.
    pub enable_fcscalp: bool,
    pub fcscalp_tp_pct: f64, // TP as fraction of notional (≈ price move %); 0.004 = 0.4%
    pub min_order: f64,      // floor entry notional (exchange min order size, e.g. $7 HYPE)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            breakout_guard: false,
            imbalance_guard: false,
            margin_sl: true,
            imbalance_ratio: 3.0,
            breakout_ntps: 300,
            sl_margin_pct: 0.7,
            seed: 0.05,
            cap: 0.90,
            cooldown_ticks: 5,
            nearest: 2,
            risk_pct: 0.40,  // risk 40% of the $0.50 allocated margin at the stop
            stop_pct: 0.007, // 0.7% stop distance (≈ margin SL at 100x)
            debug_info: 0,   // 0 = default per-event logs; see Config::debug_info for preset bits
            enable_stairs: true,
            enable_trend: true,
            // Default ON for the arbitration test branch: enable_fcscalp isn't in the group's
            // `start` signal_action params, so a live start can't pass it — default it on so the
            // FC scalp coexists with base live (the order-book edge is live-only). Backtests still
            // pass it explicitly, so this only affects live/param-absent starts.
            enable_fcscalp: true,
            fcscalp_tp_pct: 0.004,
            min_order: 0.0,
        }
    }
}

impl Config {
    pub fn apply(&mut self, values: &HashMap<String, serde_json::Value>) {
        for (k, v) in values {
            let n = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()));
            let Some(n) = n else { continue };
            match k.as_str() {
                "breakout_guard" => self.breakout_guard = n != 0.0,
                "imbalance_guard" => self.imbalance_guard = n != 0.0,
                "margin_sl" => self.margin_sl = n != 0.0,
                "imbalance_ratio" => self.imbalance_ratio = n,
                "breakout_ntps" => self.breakout_ntps = n as i64,
                "sl_margin_pct" => self.sl_margin_pct = n,
                "seed" => self.seed = n,
                "cap" => self.cap = n,
                "cooldown_ticks" => self.cooldown_ticks = n as u64,
                "nearest" => self.nearest = n as usize,
                "risk_pct" => self.risk_pct = n,
                "stop_pct" => self.stop_pct = n,
                "debug_info" => self.debug_info = n as i64,
                "enable_stairs" => self.enable_stairs = n != 0.0,
                "enable_trend" => self.enable_trend = n != 0.0,
                "enable_fcscalp" => self.enable_fcscalp = n != 0.0,
                "fcscalp_tp_pct" => self.fcscalp_tp_pct = n,
                "min_order" => self.min_order = n,
                _ => {}
            }
        }
    }
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct State {
    pub ticks: u64,
    pub ntps: i64,
    pub trandm: f64,
    pub asset_01: f64, // $ (raw × 100) to move trandm 1% — power-of-trades / depth gauge
    pub mrc_range: f64,
    pub long_close_tick: u64,
    // Cached from Indicators (not serialized)
    pub last_ind_tick: u64,
    pub has_mrc: bool,
    pub mrc_db: f64,
    pub mrc_do: f64,
    pub mrc_ds: f64,
    pub mrc_di: f64,
    pub mrc_ml: f64,
    pub mrc_ui: f64,
    pub mrc_us: f64,
    pub mrc_uo: f64,
    pub mrc_ub: f64,
    pub mrc_ml_prev: f64,
    pub angle: f64,
    // v0.55 (0.B): 5m mean-line slope + hard-break state (touch-vs-break + structure exit).
    pub mrc5_ml: f64,
    pub mrc5_ml_prev: f64,
    pub angle5: f64,    // 5m ML slope source: current − prev (sign = drift on the 5m)
    pub mrc5_break: i8, // -1 highs under 5m ML (broken down) · +1 lows over (broken up) · 0 touch/inside
    // v0.55 (0.C): stair-add ladder — most-extreme 5m cross zone we've added a stair at.
    pub long_stair_cross: i8,
    // v0.56 (0.E): has price traded through the Big band since this side opened? While false,
    // the first-reaction bracket (4 orders around Big) replaces the single mode==1 TP, because
    // we don't yet know whether Big will just-touch (revert) or fast-candle through it.
    pub long_big_tested: bool,
    // v0.55 (0.B): which strong-trandm regime the open ride belongs to (-1 sell / +1 buy / 0 none);
    // the trend-ride exits when this regime neutralizes (trandm returns toward 0).
    pub tm_side: i8,
    pub smc_dir: i8,
    pub smc_dir_prev: i8,
    // Interest line: the last MRC cross level broken — high-priority entry zone
    pub interest_cross: i8,  // which cross level was just broken through
    pub interest_dir: i8,    // +1=bullish break (add longs), -1=bearish (add shorts), 0=none
    pub interest_tests: u32, // consecutive bounces off interest level without breaking it
    pub interest_age: u64,   // 1m candles since interest was last updated
    // Volume data (updated on each 1m candle close — raw indicator values)
    pub vol_gain: f64, // Gain ratio: >0 = above baseline, <0 = below (e.g. +1.5 = 150% more)
    pub vol_gain_prev: f64, // previous candle's gain (track expanding vs shrinking)
    pub vol_gain_max: f64, // 50-candle max gain (sets "how big is big")
    pub vol_gain_min: f64, // 50-candle min gain
    pub vol_bsg: f64,  // BuySellGain: >0 = buying pressure, <0 = selling pressure
    pub vol_bsg_max: f64, // 50-candle max bsg (strongest buying seen)
    pub vol_bsg_min: f64, // 50-candle min bsg (strongest selling, negative)
    pub bearish_spike: bool, // large candle + strong selling → add short
    pub bullish_spike: bool, // large candle + strong buying → partial close short
    // BSG exhaustion candle timestamps for counter-trend entry gate (Priority 1.1)
    pub bsg_exhaustion_long_candle: u64, // tf1.candles when last buy-exhaustion (BSG>2.0) fired
    pub bsg_exhaustion_short_candle: u64, // tf1.candles when last sell-exhaustion (BSG<-2.0) fired
    // NTPS rolling history — last 10 1m values for avg/mult computation
    pub ntps_hist: [i64; 10],
    pub ntps_hist_pos: usize,
    pub ntps_hist_len: usize,
    pub ntps_avg: f64,  // rolling 10-candle average
    pub ntps_mult: f64, // size multiplier: 1.4 when rising, 0.7 at extreme peak, 1.0 otherwise
    // Indicator history (init+update, not serialized)
    pub history: IndicatorHistory,
    // Per-TF movement analysis (debug only, not serialized)
    pub tf1: TfMovState,
    pub tf5: TfMovState,
    pub tf15: TfMovState,
    pub tf30: TfMovState,
    // Consecutive 1m candles price has spent at each cross zone (index = cross+4, 0=zone-4 .. 8=zone+4)
    // High streak at zone+4 = price above UpBig for N candles; high at zone-1 = stuck at DownInner, etc.
    pub zone_streak: [u64; 9],
    // Filled level names — skip these when placing new grid orders
    pub filled: HashSet<String>,
    // Last filled band index per category (-1 = none)
    pub long_entry_idx: i8,
    pub long_tp_idx: i8,
    pub cfg: Config,
    // v0.61-signal: true once the Start SIGNAL has loaded its params into `cfg`. The Start signal
    // re-fires (backtest: on every closed position via trigger Infinity; live: re-issue), so config
    // is applied ONCE — re-applying would reset cfg back to the Start values and clobber any later
    // live `run_bot_action` config push. Config-only; never touches runtime state.
    pub cfg_loaded: bool,

    // ── Movement cascade tracker ──────────────────────────────────────────────
    // Raw block: consecutive candles with the same vol_bsg sign.
    pub mv_dir: i8,   // +1 Long, -1 Short, 0=uninit
    pub mv_high: f64, // raw block price range
    pub mv_low: f64,
    pub mv_candles: u32,
    // Cascade block: absorbs raw counter-moves < 40% of rolling avg size.
    pub mv_cas_dir: i8,
    pub mv_cas_high: f64,
    pub mv_cas_low: f64,
    pub mv_cas_candles: u32,
    // Break levels: extreme of each absorbed counter-move.
    //   Long cascade → short counter lows  (support / DCA-buy zones)
    //   Short cascade → long counter highs (resistance / DCA-sell zones)
    pub mv_breaks: [f64; 5],
    pub mv_break_n: usize,
    // Ring buffer of completed cascade block sizes → rolling avg threshold.
    pub mv_sizes: [f64; 10],
    pub mv_sizes_pos: usize,
    pub mv_sizes_len: usize,
    pub mv_avg_size: f64,

    // ── Market mode ───────────────────────────────────────────────────────────
    // 0=UNK  1=Sideways  2=TrendLong  -2=TrendShort
    pub mkt_mode: i8,
    // Which entry strategy owns the open position (see docs/strategy-arbitration.md):
    // 0=none 1=auto 2=stair 3=trend 4=trandm 5=reentry 6=fc. Exits route per owner.
    pub pos_owner: u8,
    pub mkt_mode_candles: u32, // consecutive candles in current confirmed mode
    pub mkt_cand: i8,          // pending mode candidate
    pub mkt_cand_candles: u32, // candles confirming candidate (commit at >= 2)
    pub mode_close_l: u8,      // long-side transition close stage (0=none,1=25%,2=50%,3=done)

    // ── Risk ratchet ──────────────────────────────────────────────────────────
    pub risk_init_max: f64,  // max_amount captured on Init (starting budget)
    pub risk_peak_auto: f64, // highest auto_max_amount seen since Init
    pub risk_drawdown: f64,  // entry scale factor: 1.0=normal, <1.0=drawdown

    // ── PnL MFE (max favorable excursion) — peak unrealized profit per side ──
    // Used to gate near-band TPs: only place a TP if it would be profitable AND
    // either the TP represents significant profit itself OR we've already had
    // significant profit in this position (protecting gains on reversal).
    pub long_pnl_mfe: f64,

    // ── Fast candle analytics ─────────────────────────────────────────────────
    pub fc: FastCandleStats,

    // ── Trend mode zone-based SL ──────────────────────────────────────────────
    // Cross level at which the trend SL fires.  127 = inactive (no trend position).
    // Set when a trend-s/trend-l open is emitted; cleared on position close.
    pub trend_long_sl_cross: i8,

    pub fc_tick_prev_price: f64, // price at previous SugInfo tick (for delta detection)

    // v0.56: running high/low of the CURRENT 1m candle (reset on candle close, updated each sug
    // tick). Anchors the dca-s/dca-l "sell the bounce / buy the dip" placement so a trend add rests
    // a FastCandle-distance under the candle high (or over the low) and never fills at the extreme.
    pub cur_hi: f64,
    pub cur_lo: f64,

    // ── V-movement tracker (9 per-line trackers: DB DO DS DI ML UI US UO UB) ─
    pub v_lines: [VLineTracker; 9],

    // ── Sudden-move price-run tracker (Priority 0.3, sug-tick) ───────────────
    // Detects a genuine directional move (no rollback) to justify a panic exit.
    // A "run" extends while price keeps making new extremes in run_dir, tolerating
    // shallow noise; a retracement beyond tolerance resets the run to the new dir.
    pub run_dir: i8,          // +1 = price rising, -1 = falling, 0 = uninit
    pub run_ticks: u32,       // consecutive sug ticks in the run without a rollback
    pub run_start_price: f64, // price at the run's origin (pivot)
    pub run_extreme: f64,     // furthest price reached in run_dir (low for down, high for up)

    // ── Per-side favorable-progress tracker (Priority 0.1, zone-change) ──────
    // Most-favorable 1m cross zone reached since the position opened. Used to
    // distinguish "still winning / accumulate" (hold, no TP) from "reversal"
    // (price bounced >=1 zone back toward ML → place TPs to capture profit).
    pub long_max_cross: i8, // highest cross since long opened; -127 = no long
}

impl Default for State {
    fn default() -> Self {
        Self {
            ticks: 0,
            ntps: 0,
            trandm: 0.0,
            asset_01: 0.0,
            mrc_range: 0.0,
            long_close_tick: 0,
            last_ind_tick: 0,
            has_mrc: false,
            mrc_db: 0.0,
            mrc_do: 0.0,
            mrc_ds: 0.0,
            mrc_di: 0.0,
            mrc_ml: 0.0,
            mrc_ui: 0.0,
            mrc_us: 0.0,
            mrc_uo: 0.0,
            mrc_ub: 0.0,
            mrc_ml_prev: 0.0,
            angle: 0.0,
            mrc5_ml: 0.0,
            mrc5_ml_prev: 0.0,
            angle5: 0.0,
            mrc5_break: 0,
            long_stair_cross: 0,
            long_big_tested: false,
            tm_side: 0,
            smc_dir: 0,
            smc_dir_prev: 0,
            interest_cross: 0,
            interest_dir: 0,
            interest_tests: 0,
            interest_age: 0,
            vol_gain: 0.0,
            vol_gain_prev: 0.0,
            vol_gain_max: 1.0,
            vol_gain_min: -1.0,
            vol_bsg: 0.0,
            vol_bsg_max: 1.0,
            vol_bsg_min: -1.0,
            bearish_spike: false,
            bullish_spike: false,
            bsg_exhaustion_long_candle: 0,
            bsg_exhaustion_short_candle: 0,
            ntps_hist: [0i64; 10],
            ntps_hist_pos: 0,
            ntps_hist_len: 0,
            ntps_avg: 0.0,
            ntps_mult: 1.0,
            history: IndicatorHistory::default(),
            tf1: TfMovState::default(),
            tf5: TfMovState::default(),
            tf15: TfMovState::default(),
            tf30: TfMovState::default(),
            zone_streak: [0u64; 9],
            filled: HashSet::new(),
            long_entry_idx: -1,
            long_tp_idx: -1,
            cfg: Config::default(),
            cfg_loaded: false,
            // Movement cascade
            mv_dir: 0,
            mv_high: 0.0,
            mv_low: 0.0,
            mv_candles: 0,
            mv_cas_dir: 0,
            mv_cas_high: 0.0,
            mv_cas_low: 0.0,
            mv_cas_candles: 0,
            mv_breaks: [0.0; 5],
            mv_break_n: 0,
            mv_sizes: [0.0; 10],
            mv_sizes_pos: 0,
            mv_sizes_len: 0,
            mv_avg_size: 0.0,
            // Market mode
            mkt_mode: 0,
            pos_owner: 0,
            mkt_mode_candles: 0,
            mkt_cand: 0,
            mkt_cand_candles: 0,
            mode_close_l: 0,
            // Risk ratchet
            risk_init_max: 0.0,
            risk_peak_auto: 0.0,
            risk_drawdown: 1.0,
            // PnL MFE
            long_pnl_mfe: 0.0,
            // Fast candle analytics
            fc: FastCandleStats::default(),
            fc_tick_prev_price: 0.0,
            cur_hi: 0.0,
            cur_lo: 0.0,
            // Trend mode SL (127 = inactive)
            trend_long_sl_cross: 127,
            // V-movement tracker
            v_lines: [VLineTracker::default(); 9],
            // Sudden-move price-run tracker
            run_dir: 0,
            run_ticks: 0,
            run_start_price: 0.0,
            run_extreme: 0.0,
            // Favorable-progress (-127 = no position)
            long_max_cross: -127,
        }
    }
}

pub fn save(s: &State) -> Option<serde_json::Value> {
    serde_json::to_value(StateSerde {
        ticks: s.ticks,
        long_close_tick: s.long_close_tick,
        long_entry_idx: s.long_entry_idx,
        long_tp_idx: s.long_tp_idx,
        interest_cross: s.interest_cross,
        interest_dir: s.interest_dir,
        interest_tests: s.interest_tests,
        mkt_mode: s.mkt_mode,
        mkt_mode_candles: s.mkt_mode_candles,
    })
    .ok()
}

#[derive(serde::Serialize)]
struct StateSerde {
    ticks: u64,
    long_close_tick: u64,
    long_entry_idx: i8,
    long_tp_idx: i8,
    interest_cross: i8,
    interest_dir: i8,
    interest_tests: u32,
    mkt_mode: i8,
    mkt_mode_candles: u32,
}
