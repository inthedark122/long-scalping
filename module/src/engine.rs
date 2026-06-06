use std::collections::HashMap;

use crate::debug::{debug_builder, fast_candle_debug, fc_history_debug, tf_debug, vt_debug};
use crate::fastcandle::update_fast_candles;
use crate::indicators::*;
use crate::interest::compute_entry_scale;
use crate::movement::{
    advance_market_mode, advance_tf_from_ind, init_movement, init_tf_from_ind, update_movement,
    update_risk_ratchet,
};
use crate::orders::{close, limit_order, market_open, tp_order_with_band};
use crate::vlines::{seed_v_lines, update_v_lines};
use crate::state::State;
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleEvent, ModuleInput, ModuleOpenPosition, ModuleOutput,
    ModulePlaceOrder, OrderRole,
};

use crate::consts::*;

// ── Public entry ────────────────────────────────────────────────────────────

pub fn run(input: &ModuleInput, state: &mut State) -> ModuleOutput {
    match &input.event {
        ModuleEvent::Init => handle_init(input, state),
        ModuleEvent::Indicators { timeframes } => handle_indicators(input, state, timeframes),
        ModuleEvent::SugInfo => {
            state.ticks += 1;
            handle_sug_info(input, state)
        }
        ModuleEvent::FinishPosition { direction, .. } => handle_finish(input, state, *direction),
        ModuleEvent::NewPosition { .. } => ModuleOutput {
            debug: format!(
                "new_pos t={} ln={:.1} sn={:.1}",
                state.ticks,
                input
                    .positions
                    .long
                    .as_ref()
                    .map(|p| p.notional)
                    .unwrap_or(0.0),
                input
                    .positions
                    .short
                    .as_ref()
                    .map(|p| p.notional)
                    .unwrap_or(0.0),
            ),
            ..Default::default()
        },
        ModuleEvent::OrderUpdate {
            direction,
            role,
            status,
            mark,
            ..
        } => handle_order_update(state, *direction, role, status, mark),
        ModuleEvent::Action { name, values } => handle_action(state, name, values),
        ModuleEvent::Signal { code, values, .. } => handle_signal(state, code, values),
    }
}

// ── Signal: the bot-group "Start" command (ABI b6f8fad2+ carries values) ────
// Loads the group's signal params into cfg so they actually reach the module (backtest
// action_params + live Start). Keyed off `code` and applied ONCE (see State::cfg_loaded):
// the Start signal re-fires (Infinity / re-issue) and re-applying would reset cfg back to the
// Start values, clobbering any later live config push. Config-only — never resets runtime state.
fn handle_signal(
    state: &mut State,
    code: &str,
    values: &HashMap<String, serde_json::Value>,
) -> ModuleOutput {
    if code != "start" || state.cfg_loaded {
        return ModuleOutput::default();
    }
    state.cfg.apply(values);
    state.cfg_loaded = true;
    ModuleOutput {
        debug: format!(
            "signal {} cfg seed={} cap={} cd={} near={} stairs={} trend={} dbg={}",
            code,
            state.cfg.seed,
            state.cfg.cap,
            state.cfg.cooldown_ticks,
            state.cfg.nearest,
            state.cfg.enable_stairs as u8,
            state.cfg.enable_trend as u8,
            state.cfg.debug_info,
        ),
        ..Default::default()
    }
}

// ── OrderUpdate: track filled levels by mark ───────────────────────────────

fn handle_action(
    state: &mut State,
    name: &str,
    values: &HashMap<String, serde_json::Value>,
) -> ModuleOutput {
    // force_open / force_open_long — open a LONG at market (long-only build).
    if name == "force_open" || name == "force_open_long" {
        let amt = values.get("amount").and_then(|v| v.as_f64()).unwrap_or(5.0);
        return ModuleOutput {
            open_positions: vec![market_open(Direction::Long, amt, "force")],
            debug: format!("action {} Long amt={}", name, amt),
            ..Default::default()
        };
    }
    // force_close / force_close_long — close the LONG.
    if name == "force_close" || name == "force_close_long" {
        return ModuleOutput {
            close_positions: vec![close(Direction::Long, "force_close")],
            debug: format!("action {} at t={}", name, state.ticks),
            ..Default::default()
        };
    }
    if name == "reset_fc" {
        let tick = state.ticks;
        state.fc.history.clear();
        state.fc.count = 0;
        state.fc.avg_pct = 0.0;
        state.fc.avg_gain = 0.0;
        state.fc.avg_dur = 0.0;
        state.fc.avg_calm = 0.0;
        state.fc.momentum = 0.0;
        state.fc.calm_since = tick;
        return ModuleOutput {
            debug: format!("action reset_fc at t={}", tick),
            ..Default::default()
        };
    }
    // Config loads ONLY via the Start SIGNAL (handle_signal) now — individual bot actions must
    // never mutate cfg. Re-applying config here would reset/clobber the running config (user
    // 2026-05-31: "remove cfg.apply from the OnAction because it will reset state"). Unknown
    // action names are a no-op.
    ModuleOutput {
        debug: format!(
            "action {} (ignored — config loads via Start signal only)",
            name
        ),
        ..Default::default()
    }
}

fn handle_order_update(
    state: &mut State,
    direction: Direction,
    role: &OrderRole,
    status: &str,
    mark: &Option<String>,
) -> ModuleOutput {
    if status != "Filled" {
        return ModuleOutput {
            debug: format!("order {:?} {} {:?}", direction, status, mark),
            ..Default::default()
        };
    }

    if let Some(m) = mark {
        // Transition-close stages: advance counter when each phase fills.
        if m.starts_with("tc-cl") {
            state.mode_close_l = state.mode_close_l.saturating_add(1);
        }

        if let Some(band) = m.split_once(':').map(|(_, b)| b) {
            state.filled.insert(band.to_string());
            update_fill_idx(state, direction, role, band);
        }

        return ModuleOutput {
            debug: format!(
                "fill {} {:?} {:?} filled={} idx=({},{}) cl={}",
                m,
                direction,
                role,
                state.filled.len(),
                state.long_entry_idx,
                state.long_tp_idx,
                state.mode_close_l,
            ),
            ..Default::default()
        };
    }

    ModuleOutput::default()
}

fn update_fill_idx(state: &mut State, dir: Direction, role: &OrderRole, band: &str) {
    let (arr, idx) = match (dir, role) {
        (Direction::Long, OrderRole::Entry) => (LONG_ENTRY_BANDS, &mut state.long_entry_idx),
        (Direction::Long, OrderRole::TakeProfit) => (LONG_TP_BANDS, &mut state.long_tp_idx),
        _ => return,
    };
    if let Some(pos) = arr.iter().position(|b| *b == band) {
        let i = pos as i8;
        if *idx < 0 {
            *idx = i;
        } else {
            *idx = (*idx).max(i);
        }
    }
}

// ── Init: fires once at bot start, indicators = full 10-candle history ────

fn handle_init(input: &ModuleInput, state: &mut State) -> ModuleOutput {
    state.history.init(&input.indicators);

    if let Some(bands) = read_mrc(&input.indicators) {
        apply_mrc_bands(state, &bands);
    }
    state.smc_dir = read_smc_dir(&input.indicators);

    for tf_sec in [TF, TF_5M, TF_15M, TF_30M] {
        init_tf_from_ind(&input.indicators, state, tf_sec);
    }

    // Rebuild movement cascade from last 10 candles and initialise risk ratchet.
    init_movement(&input.indicators, state);
    advance_market_mode(state);

    // v0.56 cold-start: seed the V-line tracker from the candle history (sets state.ticks to the
    // synthetic post-history tick so live ticks continue on the same scale). Depth-agnostic.
    seed_v_lines(state, &input.indicators);

    state.risk_init_max = input.max_amount;
    state.risk_peak_auto = input.auto_max_amount.max(input.max_amount);
    state.risk_drawdown = 1.0;

    ModuleOutput {
        debug: format!(
            "init t={} rng={:.2} ang={:.4} smc={} mode={} mv_cas={} avg={:.2} | 1m:{} | 5m:{} | 15m:{} | 30m:{} | vt={}{}",
            state.ticks, state.mrc_range, state.angle, state.smc_dir,
            state.mkt_mode, state.mv_cas_dir, state.mv_avg_size,
            tf_debug(&state.tf1), tf_debug(&state.tf5),
            tf_debug(&state.tf15), tf_debug(&state.tf30),
            vt_debug(&state.v_lines),
            debug_builder(state, state.cfg.debug_info & !1),
        ),
        ..Default::default()
    }
}

// ── Indicators: new candle close, indicators = only the latest candle ─────

fn handle_indicators(input: &ModuleInput, state: &mut State, timeframes: &[i64]) -> ModuleOutput {
    state.last_ind_tick = state.ticks;
    state.history.update(&input.indicators, timeframes);

    if timeframes.contains(&TF) {
        if let Some(bands) = read_mrc(&input.indicators) {
            apply_mrc_bands(state, &bands);
        }
        state.smc_dir_prev = state.smc_dir;
        state.smc_dir = read_smc_dir(&input.indicators);
        update_volume_spike(&input.indicators, state);

        // Update movement cascade and re-evaluate market mode on each 1m close.
        let c_high = read_float(&input.indicators, TF, 0, CANDLE_IND, "High").unwrap_or(0.0);
        let c_low = read_float(&input.indicators, TF, 0, CANDLE_IND, "Low").unwrap_or(0.0);
        update_movement(state, state.vol_bsg, c_high, c_low);
        advance_market_mode(state);

        // v0.56: a 1m candle just closed — reset the running high/low to the boundary price so the
        // trend-DCA anchor tracks the NEW forming candle (sug ticks extend it from here).
        state.cur_hi = input.price;
        state.cur_lo = input.price;
    }

    // v0.55 (0.B): 5m mean-line slope + hard-break state, recomputed each 5m close.
    if timeframes.contains(&TF_5M) {
        if let Some(ml5) = read_float(&input.indicators, TF_5M, 0, MRC, "MeanLine") {
            state.mrc5_ml_prev = state.mrc5_ml;
            state.mrc5_ml = ml5;
            state.angle5 = if state.mrc5_ml_prev > 0.0 {
                ml5 - state.mrc5_ml_prev
            } else {
                0.0
            };
            let hi5 = read_float(&input.indicators, TF_5M, 0, CANDLE_IND, "High").unwrap_or(ml5);
            let lo5 = read_float(&input.indicators, TF_5M, 0, CANDLE_IND, "Low").unwrap_or(ml5);
            // hard break: the whole 5m candle on one side of its ML (user: "max price under the ML")
            state.mrc5_break = if hi5 < ml5 {
                -1
            } else if lo5 > ml5 {
                1
            } else {
                0
            };
        }
    }

    for &tf_sec in timeframes {
        advance_tf_from_ind(&input.indicators, state, tf_sec);
    }

    // Market auto-open only — one-shot per candle. Limit orders live in handle_sug_info.
    let open_positions = if timeframes.contains(&TF)
        && state.has_mrc
        && (state.cfg.enable_stairs || state.cfg.enable_trend)
    {
        let ops = compute_auto_open(input, state);
        // Record trend SL cross for freshly emitted trend opens
        for op in &ops {
            match op.note.as_str() {
                "trend-l" => {
                    state.trend_long_sl_cross = state.tf1.cross - 1;
                    state.long_stair_cross = state.tf5.cross;
                }
                // v0.55 (0.C): baseline/advance the stair ladder on long opens & stairs.
                "tm-l" | "auto-l" | "re-l" | "stair-l" => state.long_stair_cross = state.tf5.cross,
                _ => {}
            }
        }
        ops
    } else {
        vec![]
    };

    ModuleOutput {
        open_positions,
        debug: format!(
            "ind t={} rng={:.2} smc={} mode={} mv={}/{:.0} | 1m:{} | 5m:{} | fc cnt={} pct={:.1}% dur={:.0}t gain={:.2} calm={:.0}c mom={:.1} pre={}{} | {}{}",
            state.ticks, state.mrc_range, state.smc_dir,
            state.mkt_mode, state.mv_cas_dir,
            state.mv_cas_high - state.mv_cas_low,
            tf_debug(&state.tf1), tf_debug(&state.tf5),
            state.fc.count, state.fc.avg_pct, state.fc.avg_dur, state.fc.avg_gain,
            state.fc.avg_calm, state.fc.momentum, state.fc.prefire,
            if state.fc.active { if state.fc.dir > 0 { " [FAST^]" } else { " [FASTv]" } } else { "" },
            fc_history_debug(&state.fc.history),
            debug_builder(state, state.cfg.debug_info),
        ),
        ..Default::default()
    }
}

// ── SugInfo: closes + reentry + grid placement ─────────────────────────────

fn handle_sug_info(input: &ModuleInput, state: &mut State) -> ModuleOutput {
    let (ntps, trandm, asset_01, fast_candle) = read_analysis(input);
    state.ntps = ntps;
    state.trandm = trandm;
    state.asset_01 = asset_01;

    // Capture previous tick price BEFORE update_fast_candles overwrites fc_tick_prev_price.
    let prev_tick_price = state.fc_tick_prev_price;

    // v0.56: extend the current 1m candle's running high/low (reset on candle close). Anchors the
    // trend-DCA "sell the bounce / buy the dip" placement.
    if state.cur_hi <= 0.0 || input.price > state.cur_hi {
        state.cur_hi = input.price;
    }
    if state.cur_lo <= 0.0 || input.price < state.cur_lo {
        state.cur_lo = input.price;
    }

    // Fast candle: real-time price delta detection on every SugInfo tick
    update_fast_candles(state, input.price);

    // V-movement: per-line crossing detection and V power/exhaustion tracking
    update_v_lines(state, input.price);

    // NTPS rolling history — track 10 candles to detect suppression vs surge
    state.ntps_hist[state.ntps_hist_pos] = ntps;
    state.ntps_hist_pos = (state.ntps_hist_pos + 1) % 10;
    if state.ntps_hist_len < 10 {
        state.ntps_hist_len += 1;
    }
    let ntps_sum: i64 = state.ntps_hist[..state.ntps_hist_len].iter().sum();
    state.ntps_avg = ntps_sum as f64 / state.ntps_hist_len as f64;
    state.ntps_mult = if state.ntps_avg < 10.0 {
        1.0
    } else {
        let r = ntps as f64 / state.ntps_avg;
        if r > 2.0 {
            0.7
        }
        // extreme spike → move may be ending, reduce size
        else if r > 1.3 {
            1.4
        }
        // rising spike → trend in motion, add more
        else {
            1.0
        } // normal / suppressed → steady
    };

    let price = input.price;
    let max = input.max_amount.max(input.auto_max_amount);
    let margin = if input.leverage > 0 {
        max / input.leverage as f64
    } else {
        max / 100.0
    };

    let long_n = input
        .positions
        .long
        .as_ref()
        .map(|p| p.notional)
        .unwrap_or(0.0);
    // UNREALIZED pnl (self-computed from mark vs avg-entry — see upnl()).
    let long_pnl = upnl(input.positions.long.as_ref(), price, true);
    let has_long = input.positions.long.is_some();
    let total_pnl = long_pnl;
    // v0.55 (0.C): reset the stair ladder when flat so each new ride builds from scratch.
    // v0.56 (0.E): track the first Big-line interaction (reset flat, set on trade-through).
    if !has_long {
        state.long_stair_cross = 0;
        state.long_big_tested = false;
    } else if state.mrc_ub > 0.0 && price >= state.mrc_ub {
        state.long_big_tested = true;
    }

    // Track peak unrealized profit (MFE — max favorable excursion).
    if has_long {
        if long_pnl > state.long_pnl_mfe {
            state.long_pnl_mfe = long_pnl;
        }
    } else {
        state.long_pnl_mfe = 0.0;
    }

    // ── Favorable-progress tracking (Priority 0.1) ──────────────────────────
    // Most-favorable 1m cross zone reached since the long opened — used to hold (no TP
    // nibble) while still winning and only TP on a reversal (>= 1 zone back toward ML).
    if has_long {
        if state.long_max_cross == -127 || state.tf1.cross > state.long_max_cross {
            state.long_max_cross = state.tf1.cross;
        }
    } else {
        state.long_max_cross = -127;
    }

    // ── Sudden-move detector (Priority 0.3) ─────────────────────────────────
    // A "run" = price moving one direction without rollback. The run extends while
    // price makes new extremes (and tolerates shallow noise within `tol`); a deeper
    // retracement resets the run to the opposite direction. A panic exit is only
    // justified by a genuine sustained move — not a single spike or slow oscillation.
    let run_tol = (state.mrc_range * 0.10).max(input.symbol_info.price_tick_size);
    if state.run_dir == 0 || prev_tick_price <= 0.0 {
        state.run_dir = if price >= prev_tick_price { 1 } else { -1 };
        state.run_ticks = 0;
        state.run_start_price = price;
        state.run_extreme = price;
    } else if state.run_dir > 0 {
        if price > state.run_extreme {
            state.run_extreme = price;
            state.run_ticks += 1;
        } else if price >= state.run_extreme - run_tol {
            state.run_ticks += 1;
        } else {
            state.run_dir = -1;
            state.run_ticks = 1;
            state.run_start_price = price;
            state.run_extreme = price;
        }
    } else {
        if price < state.run_extreme {
            state.run_extreme = price;
            state.run_ticks += 1;
        } else if price <= state.run_extreme + run_tol {
            state.run_ticks += 1;
        } else {
            state.run_dir = 1;
            state.run_ticks = 1;
            state.run_start_price = price;
            state.run_extreme = price;
        }
    }
    let run_move_pct =
        (state.run_extreme - state.run_start_price).abs() / state.run_start_price.max(1.0) * 100.0;
    let ntps_rising = (ntps as f64) > state.ntps_avg.max(1.0);
    let sudden_move = state.run_ticks >= 5 && ntps_rising && run_move_pct > 0.5;

    // Update risk ratchet before computing anything else.
    update_risk_ratchet(state, input.max_amount, input.auto_max_amount);

    // (smc_dir no longer needed here — the SMC/NTPS momentum close was removed in v0.51.0)
    let mkt_mode = state.mkt_mode;

    let mut close_positions: Vec<ModuleClosePosition> = Vec::new();

    // ── SL: last-resort margin protection ──────────────────────────────
    if state.cfg.margin_sl && total_pnl < -(margin * state.cfg.sl_margin_pct) {
        if has_long && long_n > 1.0 {
            close_positions.push(close(Direction::Long, "MARGIN SL"));
        }
    }

    // ── v0.50.0: stop-hunt sweep hold ───────────────────────────────────────
    // An up-spike INTO the upper extreme (1m cross >= 3, UpOuter/UpBig) while the 5m
    // trend is still DOWN is a liquidity grab, not a reversal — do NOT cover a short
    // there (the higher-TF trend favors holding it; covering = selling the top into the
    // sweep, right before the dump — observed 15:31). Mirror for a long into the lower
    // extreme during a 5m uptrend. These gate the noise/momentum covers ONLY; the deep
    // margin/staged SL still fires as the real backstop.
    let long_sweep_hold = state.tf1.cross <= -3 && state.tf5.regime == 2;

    // ── v0.53: trandm side-hold ─────────────────────────────────────────────
    // trandm = which side controls (-100 selling .. +100 buying). If the controlling side AGREES
    // with our open position, a counter price-move is a relief bounce / dip, NOT a flip — do NOT
    // panic-cover it; ride the side. (The deep margin SL still backstops a genuine runaway.)
    let trandm_hold_long = trandm >= TRANDM_MILD; // buying side  → hold the long through dips

    // ── NTPS momentum close — REMOVED v0.51.0 ───────────────────────────
    // Was: `if ntps>600 { close short if smc==Long; close long if smc==Short }`.
    // Removed per user (2026-05-29): the `smc` Direction is the aggressive BREAKLINE flip
    // (pullback-prone), so SMC=Long is a contrarian SHORT signal, NOT a reason to cover a
    // short — this close kept instantly killing valid shorts on the breakline flip. Exits
    // now come from the sudden-move panic (sweep-held), the margin/staged SL, and profit TPs.
    // A future SMC rework should use `DirectionSwing` (truer trend) rather than `Direction`.

    // ── Trend mode zone-based SL ─────────────────────────────────────────
    // Fires when price reverses one cross zone past the entry zone.
    // Tighter than the margin SL — cuts the loss early while the thesis is wrong.
    if state.trend_long_sl_cross != 127
        && has_long
        && long_n > 1.0
        && state.tf1.cross <= state.trend_long_sl_cross
        && !close_positions
            .iter()
            .any(|c| matches!(c.direction, Direction::Long))
    {
        close_positions.push(close(Direction::Long, "trend-sl"));
    }

    // ── Sudden-move panic close (Priority 0.3) ──────────────────────────────
    // Close only the side the move is ADVERSE to: a sharp rise threatens shorts,
    // a sharp drop threatens longs. This is the only loss-exit driven by live price
    // action (besides the deep margin SL) — small single spikes (< 0.5%) and slow
    // zone-change oscillations never trigger it (require >= 5 no-rollback ticks +
    // rising NTPS + > 0.5% move). A favorable run never closes the winning side.
    // v0.50.0: sweep-hold suppresses the panic cover when the spike is into a channel
    // extreme against the 5m trend (stop hunt) — don't sell the top / buy the bottom.
    if sudden_move {
        if state.run_dir < 0
            && has_long
            && long_n > 1.0
            && !long_sweep_hold
            && !trandm_hold_long
            && !close_positions
                .iter()
                .any(|c| matches!(c.direction, Direction::Long))
        {
            close_positions.push(close(Direction::Long, "sudden-down"));
        }
    }

    // ── v0.55 (0.B): trandm thesis-reversal exit (the trend-ride's exit; NOT a line TP) ─────
    // The ride has no fixed MRC-line TP — while the controlling side stays strong we let it FLY
    // (0.B). It exits when the strong-trandm regime it belongs to NEUTRALIZES (trandm returns
    // toward 0). P&L-agnostic: banks a winner whose momentum is done, cuts a wrong ride. The deep
    // margin SL still backstops a runaway.
    if trandm <= -TRANDM_STRONG {
        state.tm_side = -1;
    } else if trandm >= TRANDM_STRONG {
        state.tm_side = 1;
    }
    if state.tm_side == -1 && trandm > -TRANDM_MILD {
        state.tm_side = 0; // selling regime over — nothing to exit in long-only
    } else if state.tm_side == 1 && trandm < TRANDM_MILD {
        if has_long
            && long_n > 1.0
            && !close_positions
                .iter()
                .any(|c| matches!(c.direction, Direction::Long))
        {
            close_positions.push(close(Direction::Long, "tm-exit"));
        }
        state.tm_side = 0;
    }

    // ── Sug-tick organic entry (Priority Timing 0.1) ────────────────────────
    // compute_auto_open also runs on 1m candle closes; running it here lets entries
    // fire mid-candle the moment conditions align rather than waiting up to ~60s for
    // the next close. Once a position is open its internal has_long guard
    // make this a no-op, so it cannot double-open.
    let open_positions: Vec<ModuleOpenPosition> = if state.has_mrc
        && (state.cfg.enable_stairs || state.cfg.enable_trend)
    {
        let ops = compute_auto_open(input, state);
        for op in &ops {
            match op.note.as_str() {
                "trend-l" => {
                    state.trend_long_sl_cross = state.tf1.cross - 1;
                    state.long_stair_cross = state.tf5.cross;
                }
                // v0.55 (0.C): baseline/advance the stair ladder on long opens & stairs.
                "tm-l" | "auto-l" | "re-l" | "stair-l" => state.long_stair_cross = state.tf5.cross,
                _ => {}
            }
        }
        ops
    } else {
        Vec::new()
    };

    let tick = input.symbol_info.price_tick_size.max(f64::EPSILON);

    let mut place_orders = if state.has_mrc {
        compute_limit_orders(input, state)
    } else {
        vec![]
    };

    // ── Trend mode: gradual transition close for a long caught in a confirmed downtrend ────
    // Stage 0→1: 25% after 2 candles · 1→2: 25% after 5 · 2→3: 50% after 8.
    let mc = state.mkt_mode_candles;
    if mkt_mode == -2 && has_long && long_n > 1.0 {
        let (stage, min_can, pct, mark): (u8, u32, f64, &str) = match state.mode_close_l {
            0 => (0, 2, 0.25, "tc-cl1"),
            1 => (1, 5, 0.25, "tc-cl2"),
            2 => (2, 8, 0.50, "tc-cl3"),
            _ => (3, u32::MAX, 0.0, ""),
        };
        if mc >= min_can && pct > 0.0 {
            let amt = long_n * pct;
            if amt > 1.0 {
                place_orders.push(tp_order_with_band(
                    Direction::Long,
                    amt,
                    price - tick,
                    mark,
                    mark,
                ));
            }
        }
        let _ = stage;
    }

    // Priority 1.4: staged partial SL — at 50% of full SL threshold, reduce the losing side
    // by 30% via a reduce-only limit order near market price (fills on the very next tick).
    // Only fires when no full SL is already in progress (close_positions is empty).
    let early_sl_threshold = -(margin * state.cfg.sl_margin_pct * 0.50);
    if state.cfg.margin_sl && close_positions.is_empty() && total_pnl < early_sl_threshold {
        if has_long && long_pnl < 0.0 && long_n > 1.0 {
            let amt = long_n * 0.30;
            if amt > 1.0 {
                place_orders.push(tp_order_with_band(
                    Direction::Long,
                    amt,
                    price - tick,
                    "psl-l",
                    "psl-l",
                ));
            }
        }
    }

    let ord_count = place_orders.len();

    let run_arrow = if state.run_dir > 0 {
        "^"
    } else if state.run_dir < 0 {
        "v"
    } else {
        "-"
    };
    let r_side_dbg = risk_sized_notional(
        input.max_amount.max(input.auto_max_amount),
        input.leverage.max(1) as f64,
        state.cfg.risk_pct,
        state.cfg.stop_pct,
    );
    let run_dbg = format!(
        " run={}{}t {:.2}%{} lmx={} tm={:.0} a01={:.0} tms={} ls={} b5={} bgt={:.0} dmp={:.2} bk={}",
        run_arrow, state.run_ticks, run_move_pct,
        if sudden_move { " SUDDEN" } else { "" },
        state.long_max_cross,
        trandm, asset_01,
        state.tm_side, state.long_stair_cross, state.mrc5_break,
        r_side_dbg, uncertainty_damp(state),
        state.long_big_tested as u8,
    );

    ModuleOutput {
        open_positions,
        close_positions,
        place_orders,
        debug: format!(
            "sug t={} p={:.2} ntps={} avg={:.0} x{:.1} mode={} mv={}/{:.0}(a={:.0}) brk={} drf={:.2} | ln={:.1} pnl={:.4} sl={:.2} fm={:.3} ord={}{} | {} | vt={}{}",
            state.ticks, price, ntps, state.ntps_avg, state.ntps_mult,
            state.mkt_mode, state.mv_cas_dir,
            state.mv_cas_high - state.mv_cas_low, state.mv_avg_size,
            state.mv_break_n, state.risk_drawdown,
            long_n, total_pnl, -(margin * state.cfg.sl_margin_pct),
            margin - long_n / input.leverage.max(1) as f64 + total_pnl,
            ord_count,
            run_dbg,
            fast_candle_debug(&fast_candle),
            vt_debug(&state.v_lines),
            debug_builder(state, state.cfg.debug_info & !1),
        ),
        ..Default::default()
    }
}

// v0.55 (0.A): risk-based position notional. A `stop_pct` adverse move loses `notional × stop_pct`;
// bound that loss by `risk_pct` of the allocated margin (max_amount/leverage), then cap at max_amount.
// Fixes the under-sizing where a confirmed-trend entry (e.g. tm-s) rode a big move with a tiny size.
//   notional = min( (max_amount/leverage × risk_pct) / stop_pct , max_amount )
fn risk_sized_notional(max_amount: f64, leverage: f64, risk_pct: f64, stop_pct: f64) -> f64 {
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
fn budget_size(room: f64, f: f64, tilt: f64, state: &State) -> f64 {
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

// ── Auto-open: market entries, called once per 1m candle from handle_indicators ──

fn compute_auto_open(input: &ModuleInput, state: &State) -> Vec<ModuleOpenPosition> {
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

    // LONG-ONLY: open only on long setups; the bot never opens a short. (The hedge — which
    // opened the opposite side — is removed entirely along with all short entries.)
    if !has_long && !long_on_cd && cas_allows_long && !trandm_block_long {
        let (trigger, extra) = match mode {
            2 => (stair_long_ok, stair_lscale),
            1 => (cross1m <= 0 && v_bull_fresh && pzone < 0.5, 1.0),
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

fn compute_limit_orders(input: &ModuleInput, state: &State) -> Vec<ModulePlaceOrder> {
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

    // ── Spike-exit (s-ts) — REMOVED v0.51.0 ─────────────────────────────────────
    // Was a reduce-only BUY-to-cover on an SMC flip Short→Long. Removed per user
    // (2026-05-29): the `smc` Direction is the aggressive BREAKLINE flip, so a flip to
    // Long is a contrarian SHORT signal, not a reason to cover a short. After v0.48 stripped
    // its noise triggers, the SMC flip was its only trigger — so the whole block is gone.
    // Short exits now: sudden-move panic (sweep-held), margin/staged SL, profit TPs, and the
    // opposite-direction best-negative/hedge path. (Future: rebuild on `DirectionSwing`.)

    place_orders
}

// ── FinishPosition / NewPosition ───────────────────────────────────────────

fn handle_finish(input: &ModuleInput, state: &mut State, direction: Direction) -> ModuleOutput {
    // LONG-ONLY: only the long position lifecycle exists.
    if let Direction::Long = direction {
        state.long_close_tick = state.ticks;
        state
            .filled
            .retain(|k| !k.starts_with("l-") && !k.starts_with("t-l") && !k.starts_with("tc-cl"));
        state.long_entry_idx = -1;
        state.long_tp_idx = -1;
        state.mode_close_l = 0;
        state.long_pnl_mfe = 0.0;
        state.trend_long_sl_cross = 127;
    }
    ModuleOutput {
        debug: format!(
            "finish {:?} t={} p={:.2}",
            direction, state.ticks, input.price
        ),
        ..Default::default()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

// Self-computed UNREALIZED PnL (v0.49.0). `ModulePositionSummary.pnl` is REALIZED, not
// unrealized — using it as unrealized blinded the SL ladder and let the exp_pnl filter
// place below-entry "TPs". Derive unrealized directly: (mark − entry)·qty for a long,
// inverse for a short. qty is a positive magnitude.
#[inline]
fn upnl(
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
fn tp_is_profitable(entry: f64, tp_price: f64, is_short: bool, fee_pad: f64) -> bool {
    if entry <= 0.0 || tp_price <= 0.0 {
        return false;
    }
    if is_short {
        tp_price < entry - fee_pad
    } else {
        tp_price > entry + fee_pad
    }
}

fn win_mult(
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

fn compute_allocation(
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

fn compute_bias(angle: f64, pzone: f64, trandm: f64, mov5: f64) -> (f64, f64) {
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


