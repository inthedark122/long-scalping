//! Position sizing: risk-budgeted notional, uncertainty damping, and the
//! auto-open / limit-order computation plus allocation/bias helpers.

use crate::consts::*;
use crate::interest::compute_entry_scale;
use crate::orders::{limit_open, limit_order, market_open, tp_order_with_band};
use crate::state::State;
use lte_strategy_bridge::abi::{Direction, ModuleInput, ModuleOpenPosition, ModulePlaceOrder};

pub(crate) fn risk_sized_notional(max_amount: f64, leverage: f64, risk_pct: f64, stop_pct: f64) -> f64 {
    if stop_pct <= 0.0 || leverage <= 0.0 || max_amount <= 0.0 {
        return 0.0;
    }
    let alloc_margin = max_amount / leverage;
    let budget = alloc_margin * risk_pct.max(0.0); // $ accepted as loss at the stop
    (budget / stop_pct).min(max_amount)
}

// v0.56 (0.D): de-risk multiplier for an UNPREDICTABLE regime. The user keeps risk_pct=0.40
// (full base budget) but wants size cut when the read is uncertain — so we damp dynamically
// instead of lowering the base. Multiplicative: each uncertainty stacks. Clamped to [0.40, 1.0].
//   • market mode still UNKNOWN, • thin order book (asset_01 < $5M → can whip),
//   • weak 5m trend confidence, • 1m and 5m disagree on direction (cross-TF conflict).
pub(crate) fn uncertainty_damp(state: &State) -> f64 {
    let mut d = 1.0_f64;
    if state.mkt_mode == 0 {
        d *= 0.75;
    }
    if state.asset_01 > 0.0 && state.asset_01 < 50_000.0 {
        d *= 0.80;
    }
    if state.tf5.conf < 35 {
        d *= 0.85;
    }
    let (t1, t5) = (state.tf1.regime, state.tf5.regime);
    if t1 != 0 && t5 != 0 && t1.signum() != t5.signum() {
        d *= 0.85;
    }
    d.clamp(0.40, 1.0)
}

// v0.56 (0.D): per-type auto-scaling under ONE per-side risk budget. Every entry type draws a
// fraction `f` of the side's REMAINING room (= R_side − current notional) — its own curve — then
// is tilted by conviction and de-risked by uncertainty + the drawdown ratchet, and finally clamped
// to `room`. Because every add is a fraction of remaining room, the cumulative notional on a side
// converges to R_side and never overshoots (memory `feedback_risk_based_sizing`).
pub(crate) fn budget_size(room: f64, f: f64, tilt: f64, state: &State) -> f64 {
    if room <= 0.0 {
        return 0.0;
    }
    (room
        * f.clamp(0.0, 1.0)
        * tilt.max(0.0)
        * state.ntps_mult
        * uncertainty_damp(state)
        * state.risk_drawdown)
        .clamp(0.0, room)
}

// ── fcscalp price levels (order-book walls live, MRC fallback for backtest) ──────
/// Nearest order-book SUPPORT wall below `price` (live). None when there's no order
/// book (backtest) — caller then falls back to a market open at the MRC dip.
fn fcscalp_support(input: &ModuleInput, price: f64) -> Option<f64> {
    let ob = input.sug_info.as_ref().and_then(|s| s.order_book.as_ref())?;
    let support = ob
        .short_levels
        .iter()
        .map(|l| l.price)
        .filter(|&p| p > 0.0 && p < price)
        .fold(0.0_f64, f64::max); // highest wall below price = nearest support
    if support > 0.0 {
        Some(support)
    } else {
        None
    }
}

/// Up to `n` nearest order-book SUPPORT walls strictly below `ceil`, nearest first
/// (highest price first). Empty when there's no order book (backtest).
fn fcscalp_support_walls(input: &ModuleInput, ceil: f64, n: usize) -> Vec<f64> {
    let ob = match input.sug_info.as_ref().and_then(|s| s.order_book.as_ref()) {
        Some(ob) => ob,
        None => return Vec::new(),
    };
    let mut v: Vec<f64> = ob
        .short_levels
        .iter()
        .map(|l| l.price)
        .filter(|&p| p > 0.0 && p < ceil)
        .collect();
    v.sort_by(|a, b| b.total_cmp(a)); // descending → nearest-below first
    v.dedup();
    v.truncate(n);
    v
}

// ── Auto-open: market entries, called once per 1m candle from handle_indicators ──

pub(crate) fn compute_auto_open(input: &ModuleInput, state: &State) -> Vec<ModuleOpenPosition> {
    let price = input.price;
    let max = input.max_amount.max(input.auto_max_amount);
    // Priority 1.5: real-time free-margin estimate.
    // auto_max_amount only updates on position close — combine committed margin + live PnL instead.
    //   total_margin    = max budget / leverage
    //   committed_margin = open notional / leverage  (margin already locked by existing positions)
    //   free_margin_est  = total_margin − committed_margin + live_pnl
    // When free_margin_est drops near zero the balance is frozen and new opens will be rejected.
    let leverage = input.leverage.max(1) as f64;
    let total_margin = max / leverage;
    let live_pnl = upnl(input.positions.long.as_ref(), price, true)
        + upnl(input.positions.short.as_ref(), price, false);
    let open_notional = input
        .positions
        .long
        .as_ref()
        .map(|p| p.notional)
        .unwrap_or(0.0)
        + input
            .positions
            .short
            .as_ref()
            .map(|p| p.notional)
            .unwrap_or(0.0);
    let committed_margin = open_notional / leverage;
    let free_margin_est = total_margin - committed_margin + live_pnl;
    if total_margin > 0.0 && free_margin_est < total_margin * 0.10 {
        return vec![];
    }
    let has_long = input.positions.long.is_some();
    // v0.53: when the selling side is strongly in control, never open a long — ride the
    // controlling side (no fading / buying-the-dip). LONG-ONLY build (v0.61-long): shorts
    // and the hedge are removed entirely; the bot only ever opens long.
    let trandm_block_long = state.trandm <= -TRANDM_STRONG; // selling side: block long opens
    let mode = state.tf5.regime;
    let cross1m = state.tf1.cross;
    let smc_dir = match state.smc_dir {
        1 => Some(Direction::Long),
        -1 => Some(Direction::Short),
        _ => None,
    };
    let pzone = (price - state.mrc_db) / (state.mrc_ub - state.mrc_db).max(f64::EPSILON);
    let (long_alloc, _short_alloc) =
        compute_allocation(smc_dir, pzone, state.angle, state.tf5.mov, mode);
    let (lbias, _sbias) = compute_bias(state.angle, pzone, state.trandm, state.tf5.mov);
    let (lscale, _sscale) = compute_entry_scale(state, pzone);

    // v0.56 (0.D): one risk budget; every entry type draws a fraction of REMAINING room.
    let r_side = risk_sized_notional(max, leverage, state.cfg.risk_pct, state.cfg.stop_pct);
    let long_n_now = input
        .positions
        .long
        .as_ref()
        .map(|p| p.notional)
        .unwrap_or(0.0);
    let room_l = (r_side - long_n_now).max(0.0);
    // Per-type conviction fractions (each its own calc):
    let trend_strength =
        (state.tf5.mov.abs() / 100.0 * state.tf5.conf as f64 / 100.0).clamp(0.0, 1.0);
    let f_trend = 0.30 + 0.40 * trend_strength;
    let tm_conv = ((state.trandm.abs() - TRANDM_STRONG) / TRANDM_STRONG).clamp(0.0, 1.0);
    let f_tm = 0.30 + 0.30 * tm_conv;
    let angle_conv = (state.angle.abs() / 0.02).clamp(0.0, 1.0);
    let f_re = 0.25 + 0.25 * angle_conv;
    const F_SEED: f64 = 0.50; // seed leaves room for stairs to build the rest
    const F_STAIR: f64 = 0.50; // each stair takes half the remaining room (converges to R_side)

    let long_on_cd = state.long_close_tick > 0
        && state.ticks.saturating_sub(state.long_close_tick) < state.cfg.cooldown_ticks;
    let cas_allows_long = state.mv_cas_dir >= 0;

    // v0.56: Freshness = recent support test, sourced ONLY from the real-time V-line tracker.
    let now_tick = state.ticks;
    const V_CROSS_WINDOW: u64 = 6 * TICKS_PER_CANDLE; // ≈6 candles of crossing freshness
    const STAIR_CROSS_WINDOW: u64 = 12 * TICKS_PER_CANDLE; // ≈12 candles
    let v_bull_fresh = [0usize, 1, 2, 3].iter().any(|&i| {
        let vl = &state.v_lines[i];
        vl.last_cross_dir == 1
            && vl.last_cross_tick > 0
            && now_tick.saturating_sub(vl.last_cross_tick) <= V_CROSS_WINDOW
    });

    let stair_long_bid: usize = match cross1m {
        -3 => 0,
        -2 => 1,
        -1 => 2,
        0 => 3,
        1 => 5,
        2 => 6,
        3 => 7,
        4 => 8,
        _ => 9,
    };
    let cross_streak = if (-4..=4).contains(&cross1m) {
        state.zone_streak[(cross1m + 4) as usize]
    } else {
        0
    };
    let debounced = cross_streak >= 2 && cross_streak <= 12;
    let vol_exhausted =
        state.vol_gain_max > 0.5 && state.vol_gain < 0.0 && state.vol_gain < state.vol_gain_prev;

    let stair_long_ok = debounced && stair_long_bid < 9 && {
        let vl = &state.v_lines[stair_long_bid];
        vl.last_cross_dir == 1
            && vl.last_cross_tick > 0
            && now_tick.saturating_sub(vl.last_cross_tick) <= STAIR_CROSS_WINDOW
            && !vol_exhausted
            && (state.vol_bsg >= -0.1 || state.vol_gain > state.vol_gain_max * 0.3)
    };

    let gain_range_st = (state.vol_gain_max - state.vol_gain_min).max(GAIN_RANGE_FLOOR);
    let gain_pct_st = (state.vol_gain - state.vol_gain_min) / gain_range_st;
    let bsg_range_st = (state.vol_bsg_max - state.vol_bsg_min).max(BSG_RANGE_FLOOR);
    let bsg_pct_st = (state.vol_bsg - state.vol_bsg_min) / bsg_range_st;

    let stair_lscale: f64 = if stair_long_bid < 9 {
        // v0.56: A's long_side V count (resistance tests held at this line) replaces level_stats.v_bear.
        let v_mult = match state.v_lines[stair_long_bid].long_side.count {
            0 => 0.80,
            1 => 1.00,
            2 => 1.20,
            3 => 1.40,
            _ => 1.50_f64,
        };
        let vol_mult = if gain_pct_st > 0.55 && bsg_pct_st > 0.55 {
            1.30
        } else if gain_pct_st > 0.35 {
            1.10
        } else {
            0.85
        };
        (v_mult * vol_mult).clamp(0.50, 2.00)
    } else {
        1.0
    };

    let mut open_positions: Vec<ModuleOpenPosition> = Vec::new();

    // ── Fast-candle scalp (opt-in) — owns the SIDEWAYS regime ───────────────────
    // FC has its OWN entry signal (docs/strategy-arbitration.md §5b): sideways regime +
    // a fast-DOWN flush that just completed → buy the nearest order-book SUPPORT wall
    // (live) or the MRC DownInner dip (fallback/backtest). Maker LIMIT create-with-price.
    // It COEXISTS with the base strategies (no early-return): base owns the trend regime
    // (mode 2/-2), FC owns sideways (mode 1) — the base auto-l sideways arm yields below.
    if state.cfg.enable_fcscalp && !has_long && !long_on_cd && mode == 1 {
        let fc_flush = state.fc.last_end_dir == -1
            && state.fc.last_end_tick > 0
            && now_tick.saturating_sub(state.fc.last_end_tick) <= 3 * TICKS_PER_CANDLE;
        if fc_flush {
            let min_n = input
                .symbol_info
                .min_notional
                .unwrap_or(0.0)
                .max(state.cfg.min_order);
            // First grid rung — rung-sized (fcscalp_rung_frac of budget). The remaining
            // rungs are added as maker buys at lower support walls in compute_limit_orders.
            let amt = (max * state.cfg.fcscalp_rung_frac).max(min_n).max(1.0);
            if amt <= max {
                if let Some(support) = fcscalp_support(input, price) {
                    // LIVE: maker LIMIT create-with-price at the nearest support wall.
                    if support < price {
                        open_positions.push(limit_open(Direction::Long, amt, support, "fcs-l"));
                    }
                } else if state.mrc_di > state.mrc_db
                    && price <= state.mrc_di
                    && price > state.mrc_db
                {
                    // FALLBACK (backtest, no order book): MARKET open at the DownInner dip —
                    // limit opens don't fill in backtest; the TP is still a maker limit.
                    open_positions.push(market_open(Direction::Long, amt, "fcs-l"));
                }
            }
        }
    }

    // LONG-ONLY: open only on long setups; the bot never opens a short. (The hedge — which
    // opened the opposite side — is removed entirely along with all short entries.)
    if !has_long && !long_on_cd && cas_allows_long && !trandm_block_long {
        let (trigger, extra) = match mode {
            2 => (stair_long_ok, stair_lscale),
            // FC owns sideways: when enabled, base auto-l yields the mode-1 entry to FC.
            1 if !state.cfg.enable_fcscalp => {
                (cross1m <= 0 && v_bull_fresh && pzone < 0.5, 1.0)
            }
            -2 => (price <= state.mrc_db, 1.0),
            _ => (false, 1.0),
        };
        if trigger {
            // v0.56 (0.D): seed fraction of remaining room × conviction tilt, de-risked + budget-capped.
            let amt = budget_size(room_l, F_SEED, lbias * lscale * extra, state);
            let _ = long_alloc;
            if amt > 1.0 {
                open_positions.push(market_open(Direction::Long, amt, "auto-l"));
            }
        }
    }


    // ── Trend entry (zone-rejection / fast-candle breakout) — long only ─────────
    if state.cfg.enable_trend
        && !has_long
        && open_positions.is_empty()
        && !long_on_cd
        && cas_allows_long
        && !trandm_block_long
    {
        let cross_streak_now = if (-4i8..=4).contains(&cross1m) {
            state.zone_streak[(cross1m + 4) as usize]
        } else {
            0
        };
        // pzone < 0.65: never trend-long when price is already in the upper 35% of MRC range.
        let rejection = cross_streak_now >= 3
            && cross1m > 0
            && state.bullish_spike
            && state.vol_bsg > 1.0
            && pzone < 0.65;
        let breakout = state.fc.active
            && state.fc.dir == 1
            && cross1m > state.tf1.cross_prev
            && cross1m >= 2
            && pzone < 0.65;
        if rejection || breakout {
            // v0.56 (0.D): trend size scales with confirmed 5m trend strength.
            let amt = budget_size(room_l, f_trend, 1.0, state);
            if amt > 1.0 {
                open_positions.push(market_open(Direction::Long, amt, "trend-l"));
            }
        }
    }

    // ── v0.55 (0.C): trandm-long — ride the buying side on a confirmed 1m up-move ───────────
    if !has_long
        && !long_on_cd
        && cas_allows_long
        && open_positions.is_empty()
        && state.trandm >= TRANDM_STRONG
        && state.tf1.regime == 2
        && cross1m > 0
        && pzone < 0.85
    {
        // v0.56 (0.D): seed the trandm-ride sized by |trandm| conviction; stairs build the rest.
        let amt = budget_size(room_l, f_tm, 1.0, state);
        if amt > 1.0 {
            open_positions.push(market_open(Direction::Long, amt, "tm-l"));
        }
    }

    // ── v0.55 (0.C): re-enter at the 1m mean line on a soft touch, angle-confirmed — long only ──
    // The mean line is a valid entry ONLY as a soft touch (not hard-broken). The 1m-ML angle must
    // confirm the up-drift: up-angled 1m ML + buying side → long the touch.
    if !has_long && open_positions.is_empty() && state.mrc_range > 0.0 {
        let near_ml = (price - state.mrc_ml).abs() <= state.mrc_range * 0.05;
        if near_ml
            && !long_on_cd
            && cas_allows_long
            && state.trandm >= TRANDM_STRONG
            && state.angle > 0.0
        {
            // v0.56 (0.D): re-entry size scales with the 1m-ML angle (steeper drift → bigger).
            let amt = budget_size(room_l, f_re, 1.0, state);
            if amt > 1.0 {
                open_positions.push(market_open(Direction::Long, amt, "re-l"));
            }
        }
    }

    // ── v0.55 (0.C/0.A): stair-add at each new 5m up line while the strong buying side holds —
    // builds the position toward the cap on a continuation that never pulls back. Half-size
    // increments, capped by remaining room; fires once per new 5m zone (long_stair_cross advanced
    // in the note post-processing).
    if open_positions.is_empty()
        && has_long
        && state.trandm >= TRANDM_STRONG
        && state.tf5.cross > 0
        && state.tf5.cross > state.long_stair_cross
    {
        // v0.56 (0.D): each stair takes half the REMAINING budget room (diminishing → ≤ R_side).
        let stair = budget_size(room_l, F_STAIR, 1.0, state);
        if stair > 1.0 {
            open_positions.push(market_open(Direction::Long, stair, "stair-l"));
        }
    }

    open_positions
}

// ── Limit orders: entries + TPs, returned every SugInfo tick (mark = create-or-update) ──

pub(crate) fn compute_limit_orders(input: &ModuleInput, state: &State) -> Vec<ModulePlaceOrder> {
    let price = input.price;
    let tick = input.symbol_info.price_tick_size.max(f64::EPSILON);
    let max = input.max_amount.max(input.auto_max_amount);
    let min_tp_notional = (max * state.cfg.seed).max(1.0);
    let long_n = input
        .positions
        .long
        .as_ref()
        .map(|p| p.notional)
        .unwrap_or(0.0);
    // UNREALIZED pnl (self-computed — position.pnl is realized). Drives exp_pnl TP gate + free margin.
    let long_pnl = upnl(input.positions.long.as_ref(), price, true);
    let has_long = input.positions.long.is_some();
    let mode = state.tf5.regime;
    let mkt_mode = state.mkt_mode; // 0=UNK 1=Sideways 2=TrendLong -2=TrendShort
    let drf = state.risk_drawdown; // entry size scale factor from risk ratchet
    let smc_dir = match state.smc_dir {
        1 => Some(Direction::Long),
        -1 => Some(Direction::Short),
        _ => None,
    };
    let pzone = (price - state.mrc_db) / (state.mrc_ub - state.mrc_db).max(f64::EPSILON);
    let (long_alloc, _short_alloc) =
        compute_allocation(smc_dir, pzone, state.angle, state.tf5.mov, mode);
    let (lbias, _sbias) = compute_bias(state.angle, pzone, state.trandm, state.tf5.mov);
    let (lscale, _sscale) = compute_entry_scale(state, pzone);
    // v0.56 (0.D): risk budget — caps EVERY add (grid/stair) so cumulative ≤ R_side.
    let leverage = input.leverage.max(1) as f64;
    let r_side = risk_sized_notional(max, leverage, state.cfg.risk_pct, state.cfg.stop_pct);
    let lwm = win_mult(input.positions.long.as_ref(), price, true);
    let long_on_cd = state.long_close_tick > 0
        && state.ticks.saturating_sub(state.long_close_tick) < state.cfg.cooldown_ticks;

    let long_max = max * long_alloc;

    // Priority 1.5: free-margin gate — skip new entries if balance is too tight.
    // committed_margin = open notional / leverage; free_margin = budget - committed + live_pnl.
    // Block all new entries (not TPs) when free margin drops below 10% of total budget.
    let total_margin = max / leverage;
    let committed_margin = long_n / leverage;
    let free_margin = total_margin - committed_margin + long_pnl;
    let balance_ok = free_margin >= total_margin * 0.10;

    // Priority 1.2: low-NTPS directional drift gate.
    // When NTPS < 100, retail drift is likely sustained — only add DCA if price is
    // strictly above MeanLine (cross=0 at ML is not enough). High NTPS = noise,
    // mean-reversion likely → allow DCA normally.
    let drift_ok_long = state.ntps > 100 || state.tf1.cross > 0; // tightened: > 0, was >= 0

    // BSG confirmation gate (v0.28.0): do not DCA into the long when price is below ML and
    // volume confirms the move is continuing down. vol_bsg > 0 = buyers stepping in.
    let bsg_ok_long = !has_long || state.tf1.cross > 0 || state.vol_bsg > 0.0;

    let mut place_orders: Vec<ModulePlaceOrder> = Vec::new();

    // Block long DCA when price is below DownBig (stop hunt down) and not yet exhausted.
    let v_blocks_long_dca =
        state.v_lines[0].was_above == Some(false) && !state.v_lines[0].short_side.exhausted;

    // ── Fast-candle scalp (opt-in, enable_fcscalp) — LIMIT / maker only ─────────
    // Entry: rest a maker BUY at the candle low after a fast-DOWN flush ("buy the low").
    // Exit:  rest a maker reduce-only SELL at entry × (1 + fcscalp_tp_pct) ("sell the high").
    // The DownBig hard stop is the ONLY market exit (handle_sug_info) — rare, catastrophic.
    // ── Fast-candle scalp exit — runs ONLY for an FC-owned position ─────────────
    // TP = nearest order-book RESISTANCE wall above price (live) or entry×(1+tp_pct)
    // (fallback). Maker reduce-only limit. Per-owner routed: an FC position gets ONLY
    // this TP (early-return below skips base TPs); a base position skips this block.
    if state.pos_owner == OWN_FC {
        if has_long && long_n > 1.0 {
            if let Some(p) = input.positions.long.as_ref() {
                let avg = p.entry_price;
                let rung = (max * state.cfg.fcscalp_rung_frac).max(1.0);
                let grid_max = max * state.cfg.fcscalp_grid_frac;

                // ── Entry grid: ONE resting maker BUY at a time. The next rung is placed at
                //    the nearest support wall that sits ≥ fcscalp_rung_gap BELOW the last
                //    fill — so rungs are sequential (only after the prior fills, tracked via
                //    qty in handle_sug_info) and spaced out to catch a real dump instead of
                //    clustering at one price. Sideways only; capped by the grid budget.
                if mkt_mode == 1 && long_n + rung <= grid_max {
                    let anchor = if state.fc_last_fill_px > 0.0 {
                        state.fc_last_fill_px
                    } else {
                        avg
                    };
                    let ceil = anchor * (1.0 - state.cfg.fcscalp_rung_gap);
                    if let Some(&wall) = fcscalp_support_walls(input, ceil, 1).first() {
                        // single rolling mark → one resting order that trails the wall down.
                        place_orders.push(limit_order(Direction::Long, rung, wall, "fcs-g"));
                    }
                }

                // ── Exit: ONE reduce-only maker SELL at a FIXED small %% above the avg
                //    entry — do NOT trail it onto the moving order-book walls. The goal is
                //    to lock the small scalp reliably and RE-ENTER on the next dip (collect
                //    many small-%% wins), not to chase a "bigger" move that, on a small
                //    position, is tiny in $ and often misses the fill. The avg only drops as
                //    lower grid rungs fill, so this target never trails UP away from price —
                //    it only moves closer, helping it fill.
                let tp_px = avg * (1.0 + state.cfg.fcscalp_tp_pct);
                place_orders.push(tp_order_with_band(
                    Direction::Long,
                    long_n,
                    tp_px,
                    "fcs-tp",
                    "fcs-tp",
                ));
            }
        }
        return place_orders;
    }

    // ── Long TPs ─────────────────────────────────────────────────────────────
    if has_long && long_n > min_tp_notional {
        let long_budget = long_max * state.cfg.cap;
        if state.trend_long_sl_cross != 127 {
            // Trend position: 3 progressive TPs at UpSmall → UpOuter → UpBig.
            let tp3 = [
                (state.mrc_us, 0.25f64, "t-l1"),
                (state.mrc_uo, 0.30f64, "t-l2"),
                (state.mrc_ub, 0.35f64, "t-l3"),
            ];
            for (tp_price, pct, mark) in &tp3 {
                if *tp_price > price && !state.filled.contains(*mark) {
                    let exp_pnl = long_pnl + (tp_price - price) * (long_n / price.max(1.0));
                    if exp_pnl > 0.0 {
                        let amt = long_n * pct;
                        if amt > 1.0 {
                            place_orders.push(tp_order_with_band(
                                Direction::Long,
                                amt,
                                *tp_price,
                                mark,
                                mark,
                            ));
                        }
                    }
                }
            }
        } else if mode == -2 && long_n > long_budget + 10.0 {
            let trim = ((long_n - long_budget) * 0.35).min(long_n * 0.30);
            if trim > 1.0 {
                place_orders.push(tp_order_with_band(
                    Direction::Long,
                    trim,
                    price - tick,
                    "l-t1",
                    "l-ub",
                ));
            }
        } else if mode == 1 {
            // v0.56 (0.E): mirror of the short Big bracket around UpBig — STRICT profit-only per rung
            // (tp > avg entry + fee pad AND exp_pnl > 0) so stairs around UpBig never bank a loss.
            let entry_l = input
                .positions
                .long
                .as_ref()
                .map(|p| p.entry_price)
                .unwrap_or(0.0);
            let fee_pad = price * 0.0008;
            let u = state.mrc_uo;
            let b = state.mrc_ub;
            let d = (b - u).max(0.0);
            if !state.long_big_tested && d > 0.0 {
                let bracket = [
                    (b - d * 0.5, 0.20f64, "l-bk1"),  // pre-touch (below Big)
                    (b + d * 0.15, 0.30f64, "l-bk2"), // just-touch
                    (b + d * 1.0, 0.25f64, "l-bk3"),  // fast-candle 1×
                    (b + d * 2.0, 0.15f64, "l-bk4"),  // fast-candle 2×
                ];
                for (tp_price, pct, mark) in &bracket {
                    if *tp_price > price
                        && !state.filled.contains(*mark)
                        && tp_is_profitable(entry_l, *tp_price, false, fee_pad)
                    {
                        let exp_pnl = long_pnl + (tp_price - price) * (long_n / price.max(1.0));
                        if exp_pnl > 0.0 {
                            let amt = long_n * pct;
                            if amt > 1.0 {
                                place_orders.push(tp_order_with_band(
                                    Direction::Long,
                                    amt,
                                    *tp_price,
                                    mark,
                                    mark,
                                ));
                            }
                        }
                    }
                }
            } else {
                let tp_price = state.mrc_ub;
                if tp_price > price
                    && !state.filled.contains("l-ub")
                    && tp_is_profitable(entry_l, tp_price, false, fee_pad)
                {
                    let amt = long_n * 0.90;
                    if amt > 1.0 {
                        place_orders.push(tp_order_with_band(
                            Direction::Long,
                            amt,
                            tp_price,
                            "l-t1",
                            "l-ub",
                        ));
                    }
                }
            }
        }
    }

    // ── Trend-following DCA: add to position on a BOUNCE, not at the dip ─────────
    // v0.56 (live fix 2026-05-30): the old placement rested 1 tick ahead of price ("fills on the
    // very next tick move"), so during a pullback it re-shorted the dip instantly — six dca-s fills
    // at ~2004 dragged the avg to 2005 right before a rally to 2014 (−$0.12). Now we SELL THE BOUNCE:
    // rest the sell a FastCandle-distance UNDER the running candle high (the anchor trails up with new
    // highs), and ARM ONLY once price has pulled back ≥ that distance (sell_px strictly above market)
    // — so it can never fill immediately at the dip; it fills only on a bounce back toward the high.
    // Mirror for dca-l: rest a FastCandle-distance OVER the candle low (buy the dip into the low).
    // FastCandle avg move sets the distance (floored/capped to the 1m MRC range); cascade still gates.
    let fc_dca_dist = (price * state.fc.avg_pct / 100.0)
        .max(state.mrc_range * 0.05)
        .min(state.mrc_range * 0.50);
    if state.cfg.enable_stairs
        && balance_ok
        && drift_ok_long
        && bsg_ok_long
        && has_long
        && !long_on_cd
        && mkt_mode == 2
        && state.mv_cas_dir == 1
        && long_n < long_max * state.cfg.cap
        && !v_blocks_long_dca
    {
        let remaining = (r_side - long_n).max(0.0);
        let amt = (remaining * 0.15 * lbias * lwm * lscale * drf * state.ntps_mult).min(remaining);
        let buy_px = state.cur_lo.min(price) + fc_dca_dist; // over the candle low, trails down
        if amt > 1.0 && buy_px < price - tick {
            place_orders.push(limit_order(Direction::Long, amt, buy_px, "dca-l"));
        }
    }

    // ── Extended TPs for trend modes (2-3x MRC window beyond upper/lower band) ─
    let half_range = (state.mrc_ub - state.mrc_db) * 0.5;
    if mkt_mode == 2 && has_long && long_n > min_tp_notional && half_range > 0.0 {
        let ext1 = state.mrc_ub + half_range;
        let ext2 = state.mrc_ub + half_range * 2.0;
        if ext1 > price && !state.filled.contains("l-ext-t1") {
            let amt = long_n * 0.20;
            if amt > 1.0 {
                place_orders.push(tp_order_with_band(
                    Direction::Long,
                    amt,
                    ext1,
                    "l-ext-t1",
                    "l-ext-t1",
                ));
            }
        }
        if ext2 > price && !state.filled.contains("l-ext-t2") {
            let amt = long_n * 0.15;
            if amt > 1.0 {
                place_orders.push(tp_order_with_band(
                    Direction::Long,
                    amt,
                    ext2,
                    "l-ext-t2",
                    "l-ext-t2",
                ));
            }
        }
    }

    place_orders
}

// ── Helpers ─────────────────────────────────────────────────────────────────

// Self-computed UNREALIZED PnL (v0.49.0). `ModulePositionSummary.pnl` is REALIZED, not
// unrealized — using it as unrealized blinded the SL ladder and let the exp_pnl filter
// place below-entry "TPs". Derive unrealized directly: (mark − entry)·qty for a long,
// inverse for a short. qty is a positive magnitude.
#[inline]
pub(crate) fn upnl(
    pos: Option<&lte_strategy_bridge::abi::ModulePositionSummary>,
    mark: f64,
    _is_long: bool,
) -> f64 {
    // v0.52.0 FIX: `qty` is SIGNED (negative for shorts — confirmed live: a losing short read
    // as profit). So (mark − entry) · qty is correct for BOTH sides; the old long/short branch
    // double-negated the short. _is_long retained for call-site compatibility (unused).
    pos.map(|p| (mark - p.entry_price) * p.qty).unwrap_or(0.0)
}

// v0.56 (0.E): STRICT profit-only guard for a closing (TP) order — never bank a loss.
// A close is profitable only on the right side of the average entry, with a fee pad so we
// don't book a microscopic "gain" that's actually a net loss after round-trip fees + slippage.
//   SHORT (buy to close): profit ⟺ close BELOW entry − pad.   LONG (sell to close): profit ⟺ ABOVE entry + pad.
// This is the protection against the multi-order Big bracket opening a negative TP when stairs
// have moved the average entry around the Big line.
pub(crate) fn tp_is_profitable(entry: f64, tp_price: f64, is_short: bool, fee_pad: f64) -> bool {
    if entry <= 0.0 || tp_price <= 0.0 {
        return false;
    }
    if is_short {
        tp_price < entry - fee_pad
    } else {
        tp_price > entry + fee_pad
    }
}

pub(crate) fn win_mult(
    pos: Option<&lte_strategy_bridge::abi::ModulePositionSummary>,
    mark: f64,
    is_long: bool,
) -> f64 {
    let ratio = pos
        .map(|p| {
            if p.notional > 1.0 {
                upnl(Some(p), mark, is_long) / p.notional
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);
    if ratio > 0.0 {
        1.0 + (ratio * 5.0).min(0.5)
    } else {
        1.0
    }
}

pub(crate) fn compute_allocation(
    smc_dir: Option<Direction>,
    pzone: f64,
    angle: f64,
    mov5: f64,
    mode: i8,
) -> (f64, f64) {
    // Trend direction gets the majority; counter-trend is hedge-only
    let mut long_r = match mode {
        2 => 0.70_f64,  // Trend UP: primary long
        -2 => 0.30_f64, // Trend DOWN: primary short
        _ => 0.50_f64,  // Sideways/UNK: balanced
    };

    match smc_dir {
        Some(Direction::Long) => long_r += 0.05,
        Some(Direction::Short) => long_r -= 0.05,
        _ => {}
    }

    if pzone < 0.35 {
        long_r += 0.05;
    }
    if pzone > 0.65 {
        long_r -= 0.05;
    }

    if angle > 0.01 {
        long_r -= 0.05;
    }
    if angle < -0.01 {
        long_r += 0.05;
    }

    // 5m mov directional tilt: up to ±10% shift
    let mov5_bias = (mov5 / 100.0) * 0.10;
    long_r -= mov5_bias;

    long_r = long_r.clamp(0.15, 0.85);
    (long_r, 1.0 - long_r)
}

pub(crate) fn compute_bias(angle: f64, pzone: f64, trandm: f64, mov5: f64) -> (f64, f64) {
    let mut lbias = 1.0_f64;
    let mut sbias = 1.0_f64;

    if angle > 0.005 && pzone > 0.6 {
        sbias = 1.3;
        lbias = 0.8;
    }
    if angle < -0.005 && pzone < 0.4 {
        lbias = 1.3;
        sbias = 0.8;
    }

    // trandm: which side is in control (-100 selling .. +100 buying). RIDE the controlling side,
    // never fade it. (Was inverted pre-v0.53: `trandm < 30 → favor long` treated the whole
    // oversell region as a buy-the-dip — the opposite of the model and a cause of wrong-way longs.)
    if trandm <= -TRANDM_STRONG {
        sbias *= 1.3;
        lbias *= 0.6;
    } else if trandm <= -TRANDM_MILD {
        sbias *= 1.15;
        lbias *= 0.85;
    }
    if trandm >= TRANDM_STRONG {
        lbias *= 1.3;
        sbias *= 0.6;
    } else if trandm >= TRANDM_MILD {
        lbias *= 1.15;
        sbias *= 0.85;
    }

    // 5m mov order-size tilt
    if mov5 < -40.0 {
        lbias *= 0.7;
        sbias *= 1.3;
    } else if mov5 > 40.0 {
        sbias *= 0.7;
        lbias *= 1.3;
    }

    (lbias, sbias)
}


