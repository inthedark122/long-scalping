use crate::window::WindowContext;
use lte_strategy_bridge::indicator_access::IndicatorHistory;
use std::collections::{HashMap, HashSet};

// ── Movement cascade state (one instance per timeframe) ────────────────────
// A "block" = consecutive candles with the same vol_bsg sign. Small counter-moves
// (< 40% of the rolling avg block size) are absorbed into the parent cascade and
// their extreme recorded as a break level (support/resistance). Completed cascade
// block sizes feed a rolling avg = the "typical move" on that timeframe.
#[derive(Debug, Clone, Default)]
pub struct CascadeState {
    pub dir: i8,   // raw block direction: +1 Long, -1 Short, 0=uninit
    pub high: f64, // raw block price range
    pub low: f64,
    pub candles: u32,
    pub cas_dir: i8, // cascade block (absorbs sub-threshold counter-moves)
    pub cas_high: f64,
    pub cas_low: f64,
    pub cas_candles: u32,
    pub breaks: [f64; 5], // absorbed counter-move extremes (support/resistance zones)
    pub break_n: usize,
    pub sizes: [f64; 10], // ring buffer of completed cascade sizes → avg_size
    pub sizes_pos: usize,
    pub sizes_len: usize,
    pub avg_size: f64, // rolling avg block size = the "typical move" on this TF
}

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
    // FC order-book grid: each rung is `fcscalp_rung_frac` of the budget; the position
    // grids up to `fcscalp_grid_frac` of the budget across the nearest support walls.
    pub fcscalp_rung_frac: f64, // 0.075 → $15 rung at a $200 budget
    pub fcscalp_grid_frac: f64, // 0.375 → $75 max (≈5 rungs)
    pub fcscalp_rung_gap: f64,  // 0.003 → next rung ≥0.3% below the last fill (spacing)
    pub fcscalp_tp_band: f64,   // 0.02 → only ladder TPs onto walls within +2% of avg entry
    pub fcscalp_buy_zone: f64,  // 0.5 → scalp-buy when price is in the lower half of the window
    pub fcscalp_use_ob: bool,   // false → FC uses ONLY window/cascade levels (order book = disabled extension)
    pub fcscalp_rung_step: f64, // 0.4 → next rung is this fraction of the distance from the last fill to the floor
    // Early reversal/breakout long: open at the START of an up-move (not just on a dip) when price
    // crosses ABOVE the ZigZag SellPrice (the last swing-high pivot it just broke through). Maker
    // entry = a limit at that broken pivot (now support) — buy the retest, ride the pump to TP.
    pub fcscalp_zz_breakout: bool, // true → enable the ZigZag sell-cross breakout entry
    pub fcscalp_zz_seed_mult: f64, // 1.5 → breakout first entry is this × the normal dip rung (higher conviction)
    // Deep-dump hold-for-bounce: a sudden dump into the 30m deep bands is a potential FLIP zone
    // (high recovery potential). Don't force-cut at the bottom — suppress the FC cuts, refind a
    // reachable reduce-only TP near breakeven (ALLOWED to be negative), and hold for the quick
    // bounce. A sudden spike should recover suddenly; if it doesn't within deep_hold_candles, the
    // hold expires and we cut. The margin SL still backstops.
    pub fcscalp_deep_hold: bool,       // true → enable the deep-dump hold-for-bounce
    pub fcscalp_deep_cross: f64,       // -3 → "30m big deep" = tf30.cross <= this
    pub fcscalp_deep_neg_pct: f64,     // 0.005 → hold-exit TP at avg × (1 - this) (small negative ok)
    pub fcscalp_deep_hold_candles: f64, // 6 → hold this many candles for the bounce, then cut
    // Mean-line zone gate (the "when to scalp vs wait" rule). FC scalp is risky below the mean in a
    // downtrend (small TPs get trapped while price keeps falling — it bags). So FC only scalps:
    //   1. ABOVE the mean in an uptrend ("on the stairs"),
    //   2. on a PULLBACK to the mean (price sliding down from the top to ~the mean) — one bigger
    //      conviction entry (pullback_frac of grid_max) at the mean instead of dribbling in above it,
    //   3. BELOW the mean only in a clean SIDEWAYS range (mean-reversion works).
    // It WAITS otherwise (below-mean downtrend, or above-mean with no trend). Trend/stairs own the
    // below-mean zone (they can ride the recovery; the scalp can't).
    pub fcscalp_zone_gate: bool,    // true → enable the mean-line zone gate
    pub fcscalp_mean_band: f64,     // 0.10 → "around the mean" = within this × MRC range of mrc_ml
    pub fcscalp_pullback_frac: f64, // 0.5 → pullback-to-mean first order = this × grid_max (half position)
    // Stairs-zone scale-out TP: in the Momentum/stairs zone (above mean + uptrend) we EXPECT higher
    // highs, so don't cap the winner at one small movement. Split the position 1/3·1/3·1/3 — a quick
    // scalp TP, the current high (window ceiling), and a RUNNER at the expected new high (zz_sell,
    // capped at UpBig). Only while the stairs read holds; if it fades the per-rung small TPs resume.
    pub fcscalp_stairs_tp: bool, // true → enable the stairs-zone scale-out TP
    // No-worse-price re-entry guard (0.77): after an FC long closes, the time cooldown (cooldown_ticks)
    // alone let the breakout re-fire at the SAME price it just sold (close 0.3996 → re-buy 0.4000 = pointless
    // churn + taker fees). Within a short window after the exit, only re-open if price is at least this
    // fraction BELOW the exit price (a real dip); else WAIT. After the window a fresh setup re-enters freely.
    pub fcscalp_reentry_gap: f64, // 0.003 → re-entry must be ≥0.3% below the last exit within the window
    // ── Fast-return risk tracker (0.79 foundation) ──────────────────────────────
    // The strategy's edge = "sharp dips revert fast". MEASURE whether that holds right now and make it
    // the master risk gate. A SHARP down = a single candle that crosses >=2 MRC bands OR breaches below
    // DownBig (out of the channel). After one, track whether price returns to the pre-drop level within
    // fr_window candles. fr_consec_fail returns-that-didn't-come-back in a row → risk HIGH → stop new
    // entries + allow the cuts. While risk LOW we DON'T sell into the dip (suppress the cuts), letting the
    // resting TP catch the fast return. Band logic is trusted only when the MRC channel width is sane.
    pub fr_enable: bool,       // true → enable the fast-return risk tracker
    pub fr_window: f64,        // 3 → candles for a "fast" return to the pre-drop price
    pub fr_consec_fail: f64,   // 2 → this many non-returns in a row flips risk to HIGH
    pub mrc_max_width: f64,    // 0.10 → MRC bands trusted only when (UpBig-DownBig)/price < this
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
            fcscalp_rung_frac: 0.075, // ~$15 per rung at a $200 budget
            fcscalp_grid_frac: 0.375, // ~$75 max grid (≈5 rungs)
            fcscalp_rung_gap: 0.003,  // next rung ≥0.3% below the last fill
            fcscalp_tp_band: 0.02,    // ladder TPs only onto walls within +2% of avg
            fcscalp_buy_zone: 0.5,    // buy in the lower half of the window
            fcscalp_use_ob: false,    // window/cascade-only by default; order book is opt-in
            fcscalp_rung_step: 0.4,   // next rung 40% of the way to the floor (tighter near the bottom)
            fcscalp_zz_breakout: true, // catch the start of an up-move on a ZigZag sell-cross
            fcscalp_zz_seed_mult: 1.5, // breakout entry 1.5× the dip rung
            fcscalp_deep_hold: true,        // hold a sudden deep dump for the bounce, don't cut the bottom
            fcscalp_deep_cross: -3.0,       // 30m deep = bottom bands (DownOuter/DownBig)
            fcscalp_deep_neg_pct: 0.005,    // exit at ≤0.5% below avg entry (breakeven / small loss)
            fcscalp_deep_hold_candles: 6.0, // ~6 candles to bounce, else cut
            fcscalp_zone_gate: true,     // mean-line zone gate on (scalp only in favourable zones)
            fcscalp_mean_band: 0.10,     // ±10% of MRC range around the mean = "around the mean"
            fcscalp_pullback_frac: 0.5,  // pullback-to-mean first order = half the full position
            fcscalp_stairs_tp: true,     // stairs-zone scale-out TP on (ride the higher highs)
            fcscalp_reentry_gap: 0.003,  // re-entry must be ≥0.3% below the last exit (anti-churn)
            fr_enable: true,        // fast-return risk tracker on
            fr_window: 3.0,         // 3 candles for a "fast" return
            fr_consec_fail: 2.0,    // 2 non-returns in a row → risk HIGH
            mrc_max_width: 0.10,    // trust MRC bands only when channel width < 10% of price
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
                "fcscalp_rung_frac" => self.fcscalp_rung_frac = n,
                "fcscalp_grid_frac" => self.fcscalp_grid_frac = n,
                "fcscalp_rung_gap" => self.fcscalp_rung_gap = n,
                "fcscalp_tp_band" => self.fcscalp_tp_band = n,
                "fcscalp_buy_zone" => self.fcscalp_buy_zone = n,
                "fcscalp_use_ob" => self.fcscalp_use_ob = n != 0.0,
                "fcscalp_rung_step" => self.fcscalp_rung_step = n,
                "fcscalp_zz_breakout" => self.fcscalp_zz_breakout = n != 0.0,
                "fcscalp_zz_seed_mult" => self.fcscalp_zz_seed_mult = n,
                "fcscalp_deep_hold" => self.fcscalp_deep_hold = n != 0.0,
                "fcscalp_deep_cross" => self.fcscalp_deep_cross = n,
                "fcscalp_deep_neg_pct" => self.fcscalp_deep_neg_pct = n,
                "fcscalp_deep_hold_candles" => self.fcscalp_deep_hold_candles = n,
                "fcscalp_zone_gate" => self.fcscalp_zone_gate = n != 0.0,
                "fcscalp_mean_band" => self.fcscalp_mean_band = n,
                "fcscalp_pullback_frac" => self.fcscalp_pullback_frac = n,
                "fcscalp_stairs_tp" => self.fcscalp_stairs_tp = n != 0.0,
                "fcscalp_reentry_gap" => self.fcscalp_reentry_gap = n,
                "fr_enable" => self.fr_enable = n != 0.0,
                "fr_window" => self.fr_window = n,
                "fr_consec_fail" => self.fr_consec_fail = n,
                "mrc_max_width" => self.mrc_max_width = n,
                _ => {}
            }
        }
    }
}

// ── State ───────────────────────────────────────────────────────────────────

impl State {
    /// Compact decision-context string stamped onto EVERY FC order note (entry + exit), so the
    /// order itself records WHY it fired — no more reading source to explain a fill. Fields:
    ///   m{mkt_mode} r5{5m regime} posf{position-in-window 0..1} win{window width %}
    ///   pnl{total position PnL $} tdm{trandm side -100..100} {+/-}ml(price above/below mean) dh{deep-hold}
    pub fn ctx(&self, price: f64, total_pnl: f64) -> String {
        let win_pct = if price > 0.0 {
            (self.win.ceiling - self.win.floor) / price * 100.0
        } else {
            0.0
        };
        format!(
            "m{} r5{} posf{:.2} win{:.1}% pnl{:.3} tdm{:.0} {}ml dh{} | risk{} sharp{} cf{} fr{}/{} mv{}",
            self.mkt_mode,
            self.tf5.regime,
            self.win.pos_frac,
            win_pct,
            total_pnl,
            self.trandm,
            if self.mrc_ml > 0.0 && price >= self.mrc_ml {
                "+"
            } else {
                "-"
            },
            if self.fc_deep_active { 1 } else { 0 },
            if self.risk_high { "HI" } else { "lo" },
            if self.sharp_active { 1 } else { 0 },
            self.fr_consec_fail_n,
            self.fr_returns,
            self.fr_fails,
            if self.mrc_valid { 1 } else { 0 },
        )
    }

    /// Which MRC band-zone a price sits in: 0 = below DownBig, 1 = [DownBig,DownOuter), … 9 = above UpBig.
    /// = the count of band lines at-or-below the price. Used to count how many bands a candle crossed.
    /// Bands are strictly ascending (db<do<…<ub); returns 0 if bands aren't set.
    pub fn mrc_zone(&self, px: f64) -> i32 {
        if !(self.mrc_ub > self.mrc_db) {
            return 0;
        }
        let bands = [
            self.mrc_db,
            self.mrc_do,
            self.mrc_ds,
            self.mrc_di,
            self.mrc_ml,
            self.mrc_ui,
            self.mrc_us,
            self.mrc_uo,
            self.mrc_ub,
        ];
        bands.iter().filter(|&&b| px >= b).count() as i32
    }
}

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
    // SMC local swing low/high (the running local extreme — verified to track promptly, beats DP/ZZ).
    // smc_low_age / smc_high_age = candles since each last printed a NEW extreme. fc_stage = the 3-stage
    // FC regime from the low/high trend: -1 DownStairs · 0 Normal · +1 UpStairs.
    pub smc_low: f64,
    pub smc_high: f64,
    pub smc_low_age: u32,
    pub smc_high_age: u32,
    pub fc_stage: i8,
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

    // ── Movement cascade trackers (per-TF) ────────────────────────────────────
    // cas1 (1m) drives the market-mode observer + base sizing, exactly as the single
    // cascade did before. cas5 (5m) is observe-only for now: larger blocks giving the
    // bigger move magnitude / window / direction context.
    pub cas1: CascadeState,
    pub cas5: CascadeState,
    // Window context (Layer 1): recomputed each sug tick, consumed by FC (Layer 2).
    pub win: WindowContext,

    // ── ZigZag (1m) — swing pivots for the line-break entry ────────────────────
    // zz_sell = last swing-HIGH line (SellPrice). zz_dir = the "strong move" state (+1 LONG = higher
    // highs & higher lows, -1 SHORT, 0 BOTH/ranging). The entry is the LINE BREAK, not the (late)
    // strong-move confirmation: a 1m candle CLOSE that breaks above the swing-high line. Detected
    // ONLY on_indicator (the zigzag is static within a candle) and against the PREVIOUS candle's
    // SellPrice — the zigzag bumps SellPrice up the instant the line breaks, so the current value has
    // already run away. zz_break_up = that fresh break fired this candle. zz_close_prev = prior 1m
    // close (to require the break is fresh, not a continuation already above the line). Not serialized.
    pub zz_sell: f64,
    pub zz_buy: f64,
    pub zz_dir: i8,
    pub zz_klines: f64,
    pub zz_close_prev: f64,
    pub zz_break_up: bool,

    // Deep-dump hold-for-bounce: fc_deep_tick = the tick the hold armed (0 = not holding);
    // fc_deep_active = transient, set each sug tick when the hold is in force (suppresses the FC
    // force-cuts + switches sizing to the breakeven/negative reduce-only TP). Neither serialized.
    pub fc_deep_tick: u64,
    pub fc_deep_active: bool,

    // ── Market mode ───────────────────────────────────────────────────────────
    // 0=UNK  1=Sideways  2=TrendLong  -2=TrendShort
    pub mkt_mode: i8,
    // Which entry strategy owns the open position (see docs/strategy-arbitration.md):
    // 0=none 1=auto 2=stair 3=trend 4=trandm 5=reentry 6=fc. Exits route per owner.
    pub pos_owner: u8,
    // FC grid rung tracking (sequential + spaced): anchor price of the last filled rung,
    // count of filled rungs, and prev-tick qty for fill detection. Reset when flat.
    pub fc_last_fill_px: f64,
    pub fc_rungs: u8,
    pub fc_prev_qty: f64,
    // STATIC next-rung price: computed ONCE (anchored to the actual rung level + the window floor
    // at that moment) and held, so the resting `fcs-g` price does NOT wobble tick-to-tick with the
    // live price/floor (the old per-tick recompute caused same-price / rising rungs). Recomputed
    // only on (a) a fill → advance to the next rung, or (b) a partial TP while the window shifted →
    // "restart" the ladder for the new window. fc_grid_floor = the window floor it was anchored to.
    // fc_grid_dirty / fc_tp_filled are transient per-tick triggers (not serialized).
    pub fc_grid_px: f64,
    pub fc_grid_floor: f64,
    pub fc_grid_dirty: bool,
    pub fc_tp_filled: bool,
    // True once the current static rung has been emitted and is resting — guards against
    // re-emitting `fcs-g` every tick (the backtest does NOT dedup same-mark orders, so re-emission
    // stacks duplicate resting orders at one price). Cleared when the ladder recomputes (a fill /
    // restart) so the NEXT rung can be placed once.
    pub fc_grid_placed: bool,
    // Price of the resting (unfilled) FC limit open, trailed via ModuleUpdatePosition.
    // 0 = no pending open. Set when the open is emitted, cleared on fill / stale-cancel.
    pub fc_open_px: f64,
    // Last FC long EXIT price + tick (set in handle_finish). Feeds the no-worse-price re-entry guard:
    // within REENTRY_WINDOW ticks of the exit, an entry is blocked unless price is fcscalp_reentry_gap
    // below fc_exit_px (anti-churn — stops the close-then-rebuy-at-same-price round-trip).
    pub fc_exit_px: f64,
    pub fc_exit_tick: u64,
    // ── Fast-return risk tracker runtime ────────────────────────────────────────
    pub mrc_valid: bool,        // MRC channel width sane → band-crossing logic is trustworthy
    pub prev_close: f64,        // previous 1m candle close — the pre-drop reference for a sharp move
    pub sharp_active: bool,     // a sharp-down is armed and we're awaiting its return / expiry
    pub sharp_tick: u64,        // tick the sharp-down armed (for the fr_window timeout)
    pub sharp_ref_px: f64,      // price to climb back to for a "return" (the pre-drop close)
    pub sharp_bands: i32,       // how many MRC bands that sharp candle crossed down (for the note)
    pub fr_consec_fail_n: u32,  // consecutive sharp-downs that did NOT return in time
    pub fr_last_fail_px: f64,   // the breakdown level of the last non-return; price recovering above it clears risk
    pub fr_returns: u32,        // lifetime fast-returns (monitoring)
    pub fr_fails: u32,          // lifetime non-returns (monitoring)
    pub risk_high: bool,        // derived gate: reversion is failing → stop entries, allow cuts
    // Per-rung tracking for PER-ORDER FC exits: each fill records its price + qty so each
    // rung gets its own TP at rung_price × (1 + tp_dist). Reset when flat.
    pub fc_rung_px: [f64; 8],
    pub fc_rung_qty: [f64; 8],
    pub fc_rung_n: usize,
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
            smc_low: 0.0,
            smc_high: 0.0,
            smc_low_age: 9999,
            smc_high_age: 9999,
            fc_stage: 0,
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
            // Movement cascade (per-TF)
            cas1: CascadeState::default(),
            cas5: CascadeState::default(),
            win: WindowContext::default(),
            zz_sell: 0.0,
            zz_buy: 0.0,
            zz_dir: 0,
            zz_klines: 0.0,
            zz_close_prev: 0.0,
            zz_break_up: false,
            fc_deep_tick: 0,
            fc_deep_active: false,
            // Market mode
            mkt_mode: 0,
            pos_owner: 0,
            fc_last_fill_px: 0.0,
            fc_rungs: 0,
            fc_prev_qty: 0.0,
            fc_grid_px: 0.0,
            fc_grid_floor: 0.0,
            fc_grid_dirty: false,
            fc_tp_filled: false,
            fc_grid_placed: false,
            fc_open_px: 0.0,
            fc_exit_px: 0.0,
            fc_exit_tick: 0,
            mrc_valid: false,
            prev_close: 0.0,
            sharp_active: false,
            sharp_tick: 0,
            sharp_ref_px: 0.0,
            sharp_bands: 0,
            fr_consec_fail_n: 0,
            fr_last_fail_px: 0.0,
            fr_returns: 0,
            fr_fails: 0,
            risk_high: false,
            fc_rung_px: [0.0; 8],
            fc_rung_qty: [0.0; 8],
            fc_rung_n: 0,
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
