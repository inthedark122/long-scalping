use std::collections::{BTreeMap, HashMap};

use crate::orders::{close, limit_order, market_open, tp_order_with_band};
use crate::state::{State, TfMovState};
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleEvent, ModuleIndicatorValue, ModuleInput,
    ModuleOpenPosition, ModuleOutput, ModulePlaceOrder, OrderRole,
};
use lte_strategy_bridge::indicator_access::{
    IndicatorField, IndicatorFieldKey, IndicatorKey, TimeframeSec,
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
fn uncertainty_damp(state: &State) -> f64 {
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

// ── Movement cascade ─────────────────────────────────────────────────────────
//
// Ports movementCascade.ts into an incremental per-candle algorithm.
// Direction is determined by vol_bsg sign (>= 0 = Long, < 0 = Short).
// Counter-moves whose high-low size is < 40% of the rolling avg cascade size
// are absorbed: their extreme price is recorded as a break level (support/
// resistance DCA zone) and the parent cascade block continues.

pub fn update_movement(state: &mut State, vol_bsg: f64, high: f64, low: f64) {
    if high <= 0.0 || low <= 0.0 {
        return;
    }
    let dir: i8 = if vol_bsg >= 0.0 { 1 } else { -1 };

    if state.mv_dir == 0 {
        state.mv_dir = dir;
        state.mv_high = high;
        state.mv_low = low;
        state.mv_candles = 1;
        state.mv_cas_dir = dir;
        state.mv_cas_high = high;
        state.mv_cas_low = low;
        state.mv_cas_candles = 1;
        return;
    }

    if dir == state.mv_dir {
        if high > state.mv_high {
            state.mv_high = high;
        }
        if low < state.mv_low {
            state.mv_low = low;
        }
        state.mv_candles += 1;
        if dir == state.mv_cas_dir {
            if high > state.mv_cas_high {
                state.mv_cas_high = high;
            }
            if low < state.mv_cas_low {
                state.mv_cas_low = low;
            }
            state.mv_cas_candles += 1;
        }
        return;
    }

    // Raw block just completed in direction mv_dir — evaluate for cascade absorption.
    let completed_size = state.mv_high - state.mv_low;
    let threshold = if state.mv_avg_size > 0.0 {
        state.mv_avg_size * 0.40
    } else {
        f64::MAX
    };
    let is_counter = state.mv_dir != state.mv_cas_dir;
    let absorb = is_counter && completed_size < threshold;

    if absorb {
        // Small counter-move: record break level and extend cascade range.
        // mv_dir == -1 (short counter in long cascade) → break price = counter LOW (support)
        // mv_dir == +1 (long counter in short cascade) → break price = counter HIGH (resistance)
        let break_price = if state.mv_dir == -1 {
            state.mv_low
        } else {
            state.mv_high
        };
        if state.mv_break_n >= 5 {
            state.mv_breaks.rotate_left(1);
            state.mv_breaks[4] = break_price;
        } else {
            state.mv_breaks[state.mv_break_n] = break_price;
            state.mv_break_n += 1;
        }
        if state.mv_high > state.mv_cas_high {
            state.mv_cas_high = state.mv_high;
        }
        if state.mv_low < state.mv_cas_low {
            state.mv_cas_low = state.mv_low;
        }
        // First candle of the resuming cascade direction also extends the cascade.
        if dir == state.mv_cas_dir {
            if high > state.mv_cas_high {
                state.mv_cas_high = high;
            }
            if low < state.mv_cas_low {
                state.mv_cas_low = low;
            }
            state.mv_cas_candles += 1;
        }
    } else {
        // Real reversal: commit current cascade to history, start a new one.
        let cas_size = state.mv_cas_high - state.mv_cas_low;
        if cas_size > 0.0 {
            state.mv_sizes[state.mv_sizes_pos] = cas_size;
            state.mv_sizes_pos = (state.mv_sizes_pos + 1) % 10;
            if state.mv_sizes_len < 10 {
                state.mv_sizes_len += 1;
            }
            state.mv_avg_size = state.mv_sizes[..state.mv_sizes_len].iter().sum::<f64>()
                / state.mv_sizes_len as f64;
        }
        state.mv_cas_dir = dir;
        state.mv_cas_high = high.max(state.mv_high);
        state.mv_cas_low = low.min(state.mv_low);
        state.mv_cas_candles = 1;
        state.mv_break_n = 0;
        state.mv_breaks = [0.0; 5];
    }

    state.mv_dir = dir;
    state.mv_high = high;
    state.mv_low = low;
    state.mv_candles = 1;
}

/// Rebuild movement cascade from the 10-candle Init history (oldest-first pass).
pub fn init_movement(ind: &Indicators, state: &mut State) {
    for offset in (0..10usize).rev() {
        let bsg = read_float(ind, TF, offset, "volume_50", "BuySellGain").unwrap_or(0.0);
        let high = read_float(ind, TF, offset, CANDLE_IND, "High").unwrap_or(0.0);
        let low = read_float(ind, TF, offset, CANDLE_IND, "Low").unwrap_or(0.0);
        update_movement(state, bsg, high, low);
    }
}

// ── Market mode observer ─────────────────────────────────────────────────────
//
// Classifies current state into Sideways (1), TrendLong (2), TrendShort (-2).
// Requires cascade block > 1.2× avg AND at least 2 of 3 TFs agreeing.
// Mode commits after 2 consecutive confirming candles (hysteresis).

fn classify_mode(state: &State) -> i8 {
    // v0.49.0 #3: strong multi-TF trend override. A slow grind never builds a big cascade,
    // so the size_ratio gate below kept returning Sideways while 1m+5m were both clearly
    // trending and SMC agreed (observed: a clean -1% downtrend read as "sideways"). If both
    // TF regimes point the same way AND SMC confirms, it IS a trend. tf*.regime are 0/UNK
    // until warmed up (>=10 candles), so this can't fire prematurely.
    if state.tf1.regime == -2 && state.tf5.regime == -2 && state.smc_dir == -1 {
        return -2;
    }
    if state.tf1.regime == 2 && state.tf5.regime == 2 && state.smc_dir == 1 {
        return 2;
    }

    if state.mv_sizes_len < 3 || state.mv_avg_size < 0.001 {
        return 1;
    }
    let cas_size = state.mv_cas_high - state.mv_cas_low;
    let size_ratio = cas_size / state.mv_avg_size;
    if size_ratio < 1.2 {
        return 1;
    }

    let dir = state.mv_cas_dir;
    let agrees = match dir {
        1 => {
            (state.tf1.cross >= 1) as u8
                + (state.tf5.regime == 2) as u8
                + (state.tf15.regime >= 1) as u8
        }
        -1 => {
            (state.tf1.cross <= -1) as u8
                + (state.tf5.regime == -2) as u8
                + (state.tf15.regime <= -1) as u8
        }
        _ => 0,
    };
    if agrees >= 2 {
        // Sanity check: tf1.cross is real-time price zone (-4..+4).
        // A bullish cascade can lag while price has already dropped to the lower range.
        let cross_contradicts =
            (dir == 1 && state.tf1.cross <= -2) || (dir == -1 && state.tf1.cross >= 2);
        // Priority 2.1: above-avg volume candle on opposing side confirms the trend is wrong.
        // Tighter than cross_contradicts: cross <= -1 (below ML) + real volume = genuine reversal.
        let volume_contradicts = (dir == 1 && state.tf1.cross <= -1 && state.vol_gain > 0.0)
            || (dir == -1 && state.tf1.cross >= 1 && state.vol_gain > 0.0);
        // Priority 2.3: extreme market activity with strong BSG opposing the proposed trend direction.
        let ntps_contradicts = state.ntps > 400
            && ((dir == 1 && state.vol_bsg < -1.5) || (dir == -1 && state.vol_bsg > 1.5));
        if cross_contradicts || volume_contradicts || ntps_contradicts {
            1
        } else {
            dir * 2
        }
    } else {
        1
    }
}

pub fn advance_market_mode(state: &mut State) {
    let candidate = classify_mode(state);
    if candidate == state.mkt_mode {
        state.mkt_mode_candles += 1;
        state.mkt_cand = candidate;
        state.mkt_cand_candles = 0;
    } else if candidate == state.mkt_cand {
        state.mkt_cand_candles += 1;
        if state.mkt_cand_candles >= 3 {
            state.mkt_mode = candidate;
            state.mkt_mode_candles = 0;
            state.mkt_cand_candles = 0;
            // Reset transition-close progress on every mode change.
            state.mode_close_l = 0;
            state.filled.retain(|k| !k.starts_with("tc-"));
        }
    } else {
        state.mkt_cand = candidate;
        state.mkt_cand_candles = 1;
    }
}

// ── Risk ratchet ─────────────────────────────────────────────────────────────
//
// Combines two signals:
//   peak_factor: how far auto_max_amount is below its all-time high (profit drawn down)
//   floor_factor: how far auto_max_amount is below the starting max_amount (net loss)
// Both scale entry amounts; the geometric mean softens the combined effect.

pub fn update_risk_ratchet(state: &mut State, max_amount: f64, auto_max_amount: f64) {
    if max_amount <= 0.0 {
        return;
    }
    let current = if auto_max_amount > 0.0 {
        auto_max_amount
    } else {
        max_amount
    };
    if current > state.risk_peak_auto {
        state.risk_peak_auto = current;
    }

    let peak_factor = if state.risk_peak_auto > 0.0 {
        (current / state.risk_peak_auto).clamp(0.10, 1.0)
    } else {
        1.0
    };

    // Penalise only when auto_max_amount has fallen below the starting budget.
    let floor_factor = if auto_max_amount > 0.0 && auto_max_amount < max_amount {
        (auto_max_amount / max_amount).max(0.10)
    } else {
        1.0
    };

    state.risk_drawdown = (peak_factor * floor_factor).sqrt().clamp(0.10, 1.0);
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

// ── MRC / SMC helpers ───────────────────────────────────────────────────────

fn apply_mrc_bands(state: &mut State, bands: &[f64; 9]) {
    // Read ML 5 candles ago from history — works on both Init and Indicators
    // (Indicators events only carry 1 candle, so we can't use input.indicators for lookback)
    let key_ml = IndicatorFieldKey {
        timeframe: TimeframeSec::Tf1m,
        indicator: IndicatorKey::Mrc200,
        field: IndicatorField::MeanLine,
    };
    let ml_prev = match state.history.get_prev(key_ml, 5) {
        Some(ModuleIndicatorValue::Float(f)) => *f,
        _ => 0.0,
    };

    state.has_mrc = true;
    state.mrc_db = bands[0];
    state.mrc_do = bands[1];
    state.mrc_ds = bands[2];
    state.mrc_di = bands[3];
    state.mrc_ml = bands[4];
    state.mrc_ui = bands[5];
    state.mrc_us = bands[6];
    state.mrc_uo = bands[7];
    state.mrc_ub = bands[8];
    state.mrc_range = bands[8] - bands[0];
    state.mrc_ml_prev = ml_prev;
    if ml_prev > 0.0 && state.mrc_range > 0.0 {
        state.angle = (bands[4] - ml_prev) / state.mrc_range;
    }
}

fn read_smc_dir(ind: &Indicators) -> i8 {
    match read_direction(ind, TF, 0, SMC_IND, "Direction") {
        Some(Direction::Long) => 1,
        Some(Direction::Short) => -1,
        _ => 0,
    }
}

// ── Per-TF movement tracking ─────────────────────────────────────────────────

fn init_tf_from_ind(ind: &Indicators, state: &mut State, tf_sec: i64) {
    for offset in (0..10usize).rev() {
        if read_float(ind, tf_sec, offset, MRC, "MeanLine").is_none() {
            continue;
        }
        let cross = read_float(ind, tf_sec, offset, MRC, "CurrentCross")
            .map(|f| f.round() as i8)
            .unwrap_or(0);
        let candle_dir = match read_direction(ind, tf_sec, offset, CANDLE_IND, "Direction") {
            Some(Direction::Long) => 1i8,
            Some(Direction::Short) => -1i8,
            _ => 0i8,
        };
        {
            let tf_state = match tf_sec {
                TF => &mut state.tf1,
                TF_5M => &mut state.tf5,
                TF_15M => &mut state.tf15,
                TF_30M => &mut state.tf30,
                _ => return,
            };
            advance_tf(tf_state, cross, candle_dir, 0.0);
        }
        // Populate level stats, interest line, and zone streaks from history
        // so the first SugInfo can immediately evaluate stair/V-bounce conditions
        if tf_sec == TF {
            update_interest_line(state);
            update_zone_streak(state);
        }
    }
}

fn advance_tf_from_ind(ind: &Indicators, state: &mut State, tf_sec: i64) {
    let cross = match read_float(ind, tf_sec, 0, MRC, "CurrentCross") {
        Some(f) => f.round() as i8,
        None => return,
    };
    let candle_dir = match read_direction(ind, tf_sec, 0, CANDLE_IND, "Direction") {
        Some(Direction::Long) => 1i8,
        Some(Direction::Short) => -1i8,
        _ => 0i8,
    };
    {
        let tf_state = match tf_sec {
            TF => &mut state.tf1,
            TF_5M => &mut state.tf5,
            TF_15M => &mut state.tf15,
            TF_30M => &mut state.tf30,
            _ => return,
        };
        advance_tf(tf_state, cross, candle_dir, 0.0);
    }
    // After 1m candle: update interest line, per-level statistics, zone streaks
    if tf_sec == TF {
        update_interest_line(state);
        update_zone_streak(state);
    }
}

fn advance_tf(tf: &mut TfMovState, cross: i8, candle_dir: i8, ext: f64) {
    let prev_cross = tf.cross;
    tf.cross_prev = prev_cross;
    tf.candles += 1;
    tf.cross = cross;

    tf.history[tf.hist_pos] = cross;
    tf.hist_pos = (tf.hist_pos + 1) % 20;
    if tf.hist_len < 20 {
        tf.hist_len += 1;
    }

    // Floor/ceiling from full ring buffer
    let (mut mn, mut mx) = (cross, cross);
    for i in 0..tf.hist_len {
        let idx = (tf.hist_pos + 20 - 1 - i) % 20;
        let v = tf.history[idx];
        if v < mn {
            mn = v;
        }
        if v > mx {
            mx = v;
        }
    }
    let old_floor = tf.floor;
    let old_ceiling = tf.ceiling;
    tf.floor = mn;
    tf.ceiling = mx;
    tf.floor_age = if mn != old_floor || tf.candles == 1 {
        0
    } else {
        tf.floor_age + 1
    };
    tf.ceiling_age = if mx != old_ceiling || tf.candles == 1 {
        0
    } else {
        tf.ceiling_age + 1
    };

    // Angle sign: track consecutive cross movement direction
    let delta = cross.saturating_sub(prev_cross).signum();
    if delta != 0 {
        if delta == tf.angle_sign {
            tf.angle_sign_candles += 1;
        } else {
            tf.angle_sign = delta;
            tf.angle_sign_candles = 1;
        }
    }

    // Slow mov update: ±step per candle based on cross level, body/shadow, extension
    let dir_sign = cross.signum() as f64;
    let abs_cross = cross.abs() as f64;
    let body_factor: f64 =
        if candle_dir != 0 && ((candle_dir > 0 && cross > 0) || (candle_dir < 0 && cross < 0)) {
            1.5 // body agrees with cross = confirmed directional cross
        } else if candle_dir != 0 {
            0.7 // body opposes = shadow cross = weaker
        } else {
            1.0
        };
    let ext_bonus = ext.abs().min(3.0);
    let step = dir_sign * (1.0 + abs_cross * 0.8 + ext_bonus) * body_factor;
    let momentum = if (tf.mov > 20.0 && step > 0.0) || (tf.mov < -20.0 && step < 0.0) {
        1.3
    } else if (tf.mov > 0.0 && step < 0.0) || (tf.mov < 0.0 && step > 0.0) {
        0.5
    } else {
        1.0
    };
    tf.mov = (tf.mov + step * momentum).clamp(-100.0, 100.0);
    if cross == 0 {
        tf.mov *= 0.85;
    }

    tf.regime = classify_regime(tf);
    tf.conf = compute_conf(tf);
}

fn update_volume_spike(ind: &Indicators, state: &mut State) {
    let gain = read_float(ind, TF, 0, "volume_50", "Gain").unwrap_or(0.0);
    let gain_min = read_float(ind, TF, 0, "volume_50", "GainMin").unwrap_or(-1.0);
    let gain_max = read_float(ind, TF, 0, "volume_50", "GainMax").unwrap_or(1.0);
    let bsg = read_float(ind, TF, 0, "volume_50", "BuySellGain").unwrap_or(0.0);
    let bsg_min = read_float(ind, TF, 0, "volume_50", "BuySellGainMin").unwrap_or(-1.0);
    let bsg_max = read_float(ind, TF, 0, "volume_50", "BuySellGainMax").unwrap_or(1.0);

    state.vol_gain_prev = state.vol_gain;
    state.vol_gain = gain;
    state.vol_gain_max = gain_max;
    state.vol_gain_min = gain_min;
    state.vol_bsg = bsg;
    state.vol_bsg_max = bsg_max;
    state.vol_bsg_min = bsg_min;

    // Spike detection: normalize within 50-candle range to find outlier candles
    let gain_range = (gain_max - gain_min).max(GAIN_RANGE_FLOOR);
    let bsg_range = (bsg_max - bsg_min).max(BSG_RANGE_FLOOR);
    let gain_pct = (gain - gain_min) / gain_range; // 1.0 = biggest candle in 50c
    let bsg_pct = (bsg - bsg_min) / bsg_range; // 0.0 = max selling, 1.0 = max buying

    // Large candle (top half of 50c range) + strong directional pressure
    state.bearish_spike = gain_pct > 0.50 && bsg_pct < 0.30;
    state.bullish_spike = gain_pct > 0.50 && bsg_pct > 0.70;

    // Priority 1.1: track exhaustion candles (BSG > 2.0 = climax buy; BSG < -2.0 = climax sell)
    if state.bullish_spike && bsg > 2.0 {
        state.bsg_exhaustion_long_candle = state.tf1.candles;
    }
    if state.bearish_spike && bsg < -2.0 {
        state.bsg_exhaustion_short_candle = state.tf1.candles;
    }
}

// ── V-movement tracker ───────────────────────────────────────────────────────
//
// Runs on every SugInfo tick. One VLineTracker per MRC line — each tracks its
// own V history permanently (no reset between Vs). This preserves V1/V2/V3
// power history even when price oscillates through adjacent lines between spikes.
fn update_v_lines(state: &mut State, price: f64) {
    if !state.has_mrc || state.mrc_range <= 0.0 {
        return;
    }
    let levels: [f64; 9] = [
        state.mrc_db,
        state.mrc_do,
        state.mrc_ds,
        state.mrc_di,
        state.mrc_ml,
        state.mrc_ui,
        state.mrc_us,
        state.mrc_uo,
        state.mrc_ub,
    ];
    let range = state.mrc_range;
    let tick = state.ticks; // sug tick for real-time crossing recency (Timing 0.2)
    apply_v_lines(state, &levels, range, price, tick);
}

// Core V-line update: process ONE price point against a given set of 9 line values on a given tick.
// Used both live (current bands, state.ticks) and for cold-start seeding (historical bands per
// candle, synthetic ticks). Maintains per-line peak/trough, completed-V power, and crossing recency.
fn apply_v_lines(state: &mut State, levels: &[f64; 9], range: f64, price: f64, tick: u64) {
    if range <= 0.0 {
        return;
    }
    for (i, &lvl) in levels.iter().enumerate() {
        if lvl <= 0.0 {
            continue;
        }
        let now_above = price >= lvl;
        let vl = &mut state.v_lines[i];

        match vl.was_above {
            None => {
                // First tick: initialise without closing any V
                vl.was_above = Some(now_above);
                if now_above {
                    vl.peak = Some(price);
                } else {
                    vl.trough = Some(price);
                }
            }
            Some(prev_above) if prev_above != now_above => {
                // Crossing — record real-time recency/direction (feeds entry freshness)
                vl.last_cross_tick = tick;
                vl.last_cross_dir = if now_above { 1 } else { -1 };
                if now_above {
                    // Crossed above: close the short V that was in progress
                    if let Some(tr) = vl.trough {
                        let power = (lvl - tr).abs() / range * 100.0;
                        let s = &mut vl.short_side;
                        s.prev_power = s.last_power;
                        s.last_power = power;
                        s.count = s.count.saturating_add(1);
                        s.exhausted = s.count >= 2 && s.last_power < s.prev_power * 0.70;
                        s.stair = s.count >= 2 && s.last_power > s.prev_power * 1.10;
                    }
                    vl.trough = None;
                    vl.peak = Some(price);
                } else {
                    // Crossed below: close the long V that was in progress
                    if let Some(pk) = vl.peak {
                        let power = (pk - lvl).abs() / range * 100.0;
                        let s = &mut vl.long_side;
                        s.prev_power = s.last_power;
                        s.last_power = power;
                        s.count = s.count.saturating_add(1);
                        s.exhausted = s.count >= 2 && s.last_power < s.prev_power * 0.70;
                        s.stair = s.count >= 2 && s.last_power > s.prev_power * 1.10;
                    }
                    vl.peak = None;
                    vl.trough = Some(price);
                }
                vl.was_above = Some(now_above);
            }
            Some(_) => {
                // Same side: update peak or trough
                if now_above {
                    vl.peak = Some(vl.peak.map_or(price, |p| p.max(price)));
                } else {
                    vl.trough = Some(vl.trough.map_or(price, |t| t.min(price)));
                }
            }
        }
    }
}

// v0.56 cold-start: seed the V-line tracker from the candle history available at Init so the
// freshness / stair / asymmetry signals aren't blind during the live warm-up. DEPTH-AGNOSTIC —
// it uses however many candles the feed provides (10 today, more later) by scanning until the MRC
// history runs out. Each candle's High/Low/Close is replayed against THAT candle's own MRC bands on
// a synthetic tick timeline (~TICKS_PER_CANDLE per candle); live ticks then continue on the same
// scale so the freshness windows age correctly. Candle wicks (High/Low) yield real penetration
// power, so with more candles the V counts / exhaustion populate fully.
fn seed_v_lines(state: &mut State, ind: &Indicators) {
    const MAX_SEED_CANDLES: usize = 512; // ready for far more than 10 when the feed sends them
    let mut depth = 0usize;
    for offset in 0..MAX_SEED_CANDLES {
        if read_mrc_at(ind, offset).is_none() {
            break;
        }
        depth = offset + 1;
    }
    if depth == 0 {
        return;
    }

    for k in 0..depth {
        let offset = depth - 1 - k; // oldest → newest
        let synthetic_tick = (k as u64 + 1) * TICKS_PER_CANDLE; // start at 1 candle so tick > 0
        let bands = match read_mrc_at(ind, offset) {
            Some(b) => b,
            None => continue,
        };
        let range = bands[8] - bands[0];
        if range <= 0.0 {
            continue;
        }
        let ml = bands[4];
        let high = read_float(ind, TF, offset, CANDLE_IND, "High").unwrap_or(ml);
        let low = read_float(ind, TF, offset, CANDLE_IND, "Low").unwrap_or(ml);
        let close = read_float(ind, TF, offset, CANDLE_IND, "Close").unwrap_or(ml);
        // Replay the candle's extremes in temporal order (by direction), then the close, so upper/
        // lower wick rejections register as real Vs.
        let path = match read_direction(ind, TF, offset, CANDLE_IND, "Direction") {
            Some(Direction::Long) => [low, high, close], // up candle: dipped then rallied
            _ => [high, low, close],                     // down/unknown: rallied then dropped
        };
        for &px in &path {
            apply_v_lines(state, &bands, range, px, synthetic_tick);
        }
    }
    // Hand off to live ticks on the same synthetic scale (newest seeded candle = depth·TPC).
    state.ticks = (depth as u64) * TICKS_PER_CANDLE;
}

// ── Fast candle tracker ──────────────────────────────────────────────────────
//
// Runs on every SugInfo tick using real-time price delta.
// Groups consecutive fast-moving ticks (same direction) into one "event".
// A fast tick = price moved > 0.2% of MRC range in one tick.
// BSG/Gain from the last closed 1m candle enriches peak stats.
// Pause tolerance: 5 slow ticks before ending the group (SugInfo is noisy).
// All statistics use EMA(α=0.20).
fn update_fast_candles(state: &mut State, price: f64) {
    let tick = state.ticks;
    let range = state.mrc_range;
    if range <= 0.0 {
        state.fc_tick_prev_price = price;
        return;
    }

    let prev = state.fc_tick_prev_price;
    // On first call or after a reset, just store price
    if prev == 0.0 {
        state.fc_tick_prev_price = price;
        return;
    }

    let delta = price - prev;
    state.fc_tick_prev_price = price;

    // A tick is "fast" when price moved > 0.2% of MRC range in one SugInfo step
    let fast_threshold = range * 0.002;
    let fast_bull = delta > fast_threshold;
    let fast_bear = delta < -fast_threshold;
    let is_fast = fast_bull || fast_bear;
    let dir_now: i8 = if fast_bull { 1 } else { -1 };

    // Pre-event signal: BSG-based (uses last closed 1m candle — intentionally stale).
    // High volume + ambiguous direction → often precedes a directional spike.
    let gain_range = (state.vol_gain_max - state.vol_gain_min).max(GAIN_RANGE_FLOOR);
    let bsg_range = (state.vol_bsg_max - state.vol_bsg_min).max(BSG_RANGE_FLOOR);
    let gain_pct = (state.vol_gain - state.vol_gain_min) / gain_range;
    let bsg_pct = (state.vol_bsg - state.vol_bsg_min) / bsg_range;
    let prefire_bull = gain_pct > 0.55 && (0.40..0.65).contains(&bsg_pct) && state.vol_bsg > 0.0;
    let prefire_bear = gain_pct > 0.55 && (0.35..0.60).contains(&bsg_pct) && state.vol_bsg < 0.0;
    let fc = &mut state.fc;
    if prefire_bull {
        fc.prefire = 1;
        fc.prefire_candle = tick;
    } else if prefire_bear {
        fc.prefire = -1;
        fc.prefire_candle = tick;
    } else if tick > fc.prefire_candle + 30 {
        fc.prefire = 0;
    } // expire after ~30 ticks

    const ALPHA: f64 = 0.20;
    let ema = |cur: f64, val: f64| -> f64 {
        if cur == 0.0 {
            val
        } else {
            cur * (1.0 - ALPHA) + val * ALPHA
        }
    };

    if fc.active {
        if is_fast && dir_now == fc.dir {
            // Enrich with BSG/Gain from last candle when available
            fc.peak_gain = fc.peak_gain.max(state.vol_gain.abs());
            fc.peak_bsg = fc.peak_bsg.max(state.vol_bsg.abs());
            fc.pause = 0;
        } else if !is_fast && fc.pause < 5 {
            fc.pause += 1; // tolerate brief pauses
        } else {
            // Event ends — update rolling stats (all inputs clamped to prevent outlier drift)
            let pct = ((price - fc.start_price).abs() / fc.start_price.max(f64::EPSILON) * 100.0)
                .min(5.0); // cap at 5%
            let dur = tick.saturating_sub(fc.start_candle).min(60) as f64; // cap at 60 ticks
            let calm = fc.start_candle.saturating_sub(fc.calm_since).min(300) as f64; // cap at 300 ticks (~5 min quiet)
            let gain = fc.peak_gain.min(10.0); // cap gain at 10
            fc.avg_pct = ema(fc.avg_pct, pct);
            fc.avg_gain = ema(fc.avg_gain, gain);
            fc.avg_dur = ema(fc.avg_dur, dur);
            fc.avg_calm = ema(fc.avg_calm, calm.max(1.0));
            fc.count += 1;
            fc.momentum = (fc.momentum * 0.85 + fc.dir as f64).clamp(-10.0, 10.0);
            fc.active = false;
            fc.calm_since = tick;
            // Sliding window — keep last 8
            fc.history.push(crate::state::FcEvent {
                dir: fc.dir,
                pct,
                dur: dur as u64,
            });
            if fc.history.len() > 8 {
                fc.history.remove(0);
            }
            fc.pause = 0;
        }
    } else if is_fast {
        fc.active = true;
        fc.dir = dir_now;
        fc.start_price = price;
        fc.start_candle = tick;
        fc.peak_gain = state.vol_gain.abs();
        fc.peak_bsg = state.vol_bsg.abs();
        fc.pause = 0;
    } else if tick % 50 == 0 {
        fc.momentum *= 0.95; // slow decay during calm periods
    }
}

// ── Interest line ────────────────────────────────────────────────────────────
//
// Tracks which MRC cross zone is the current "key level" — the level price
// has been repeatedly testing without breaking. The more tests without a break,
// the stronger the signal in that direction.
//
// Logic:
//   - delta > 0 (cross moved up) and aligns with interest_dir → advance frontier
//   - delta > 0 but opposes interest_dir (e.g. brief bounce) → count as test
//   - penetration >1 zone opposite → interest flipped, new direction
fn update_interest_line(state: &mut State) {
    let cross = state.tf1.cross;
    let prev = state.tf1.cross_prev;

    if cross == prev {
        state.interest_age = state.interest_age.saturating_add(1);
        return;
    }

    let delta = (cross - prev).signum() as i8;

    if state.interest_dir == 0 {
        state.interest_cross = cross;
        state.interest_dir = delta;
        state.interest_age = 0;
        return;
    }

    if delta == state.interest_dir {
        // Continuing in the same direction: advance the frontier
        if (delta > 0 && cross > state.interest_cross)
            || (delta < 0 && cross < state.interest_cross)
        {
            state.interest_cross = cross;
            state.interest_tests = 0; // moved past old level
        }
        state.interest_age += 1;
    } else {
        // Moving opposite — measure how deep the counter-move is
        let depth: i8 = match state.interest_dir {
            1 => (state.interest_cross - cross).max(0),
            _ => (cross - state.interest_cross).max(0),
        };

        if depth <= 1 {
            // Shallow bounce: treat as a test of the interest level
            state.interest_tests = state.interest_tests.saturating_add(1);
            state.interest_age += 1;
        } else {
            // Penetrated 2+ zones: interest broken, flip direction
            state.interest_cross = cross;
            state.interest_dir = delta;
            state.interest_age = 0;
            state.interest_tests = 0;
        }
    }
}

// Maintain zone_streak: consecutive 1m candles price has spent in each cross zone.
// Called on every 1m candle close (after advance_tf updates cross/cross_prev).
fn update_zone_streak(state: &mut State) {
    let cross = state.tf1.cross;
    let prev = state.tf1.cross_prev;

    let cidx = (cross + 4) as usize;
    if cidx >= 9 {
        return;
    }

    if cross == prev {
        state.zone_streak[cidx] += 1;
    } else {
        // Reset old zone, start fresh in new zone
        let pidx = (prev + 4) as usize;
        if pidx < 9 {
            state.zone_streak[pidx] = 0;
        }
        state.zone_streak[cidx] = 1;
    }
}

// ── Entry scale from interest + regime + asymmetry ───────────────────────────
//
// Returns (long_scale, short_scale) multipliers applied to all entry amounts.
//
// Interest line boost: when price has tested a level N times without breaking,
// entries in the confirmed direction scale up proportionally.
//
// Strong-side asymmetry: one side breaks extreme bands while the other is stuck
// at a near-ML level. Example: price above UpBig for 5+ candles (zone_streak[8])
// while short side can't reach DownBig (DownBig never crossed, price drifting near
// DownInner) → market has a strong directional bias, entries follow it.
//
// Sideways dead zone: WIN regime near MeanLine (pzone 0.38-0.62) — cut entries
// opposing the SMC direction to avoid fading a ranging market.
fn compute_entry_scale(state: &State, pzone: f64) -> (f64, f64) {
    let mut lscale = 1.0_f64;
    let mut sscale = 1.0_f64;

    // ── Interest line boost ───────────────────────────────────────────────
    let itests = state.interest_tests;
    if itests >= 3 {
        let boost = ((itests - 2) as f64 * 0.20).min(0.80);
        match state.interest_dir {
            -1 => {
                sscale *= 1.0 + boost;
                lscale *= (1.0 - boost * 0.5).max(0.30);
            }
            1 => {
                lscale *= 1.0 + boost;
                sscale *= (1.0 - boost * 0.5).max(0.30);
            }
            _ => {}
        }
    }

    // ── Strong-side asymmetry ─────────────────────────────────────────────
    // zone_streak[8] = candles above UpBig; zone_streak[0] = candles below DownBig.
    // v0.56: extreme-line break age now from A's v_lines last_cross_tick (ticks, ≈15/candle) —
    // level_stats (B) removed. zone_streak (candle-based) still anchors this, bounding the drift.
    let now_tick = state.ticks;
    const RECENT_BREAK_TICKS: u64 = 30 * TICKS_PER_CANDLE; // ≈30 candles
    const STALE_BREAK_TICKS: u64 = 60 * TICKS_PER_CANDLE; // ≈60 candles
    let ub_streak = state.zone_streak[8]; // consecutive candles at cross=+4 (above UpBig)
    let db_streak = state.zone_streak[0]; // consecutive candles at cross=-4 (below DownBig)
    let ub_cross_age = if state.v_lines[8].last_cross_tick > 0 {
        now_tick.saturating_sub(state.v_lines[8].last_cross_tick)
    } else {
        u64::MAX
    };
    let db_cross_age = if state.v_lines[0].last_cross_tick > 0 {
        now_tick.saturating_sub(state.v_lines[0].last_cross_tick)
    } else {
        u64::MAX
    };

    // Strong long: price has been above UpBig (or recently broke through it upward)
    //              while DownBig was never / very long ago crossed downward
    let strong_long = (ub_streak >= 5
        || (ub_cross_age < RECENT_BREAK_TICKS && state.v_lines[8].last_cross_dir == 1))
        && db_cross_age > STALE_BREAK_TICKS;
    // Strong short: symmetric — price below DownBig while UpBig is stale
    let strong_short = (db_streak >= 5
        || (db_cross_age < RECENT_BREAK_TICKS && state.v_lines[0].last_cross_dir == -1))
        && ub_cross_age > STALE_BREAK_TICKS;

    if strong_long {
        lscale *= 1.40;
        sscale *= 0.50;
    } else if strong_short {
        sscale *= 1.40;
        lscale *= 0.50;
    }

    // ── Sideways dead zone ────────────────────────────────────────────────
    if state.tf1.regime == 1 && pzone > 0.38 && pzone < 0.62 {
        match state.smc_dir {
            1 => sscale *= 0.35,
            -1 => lscale *= 0.35,
            _ => {
                lscale *= 0.60;
                sscale *= 0.60;
            }
        }
    }

    (lscale.max(0.10), sscale.max(0.10))
}

fn classify_regime(tf: &TfMovState) -> i8 {
    if tf.hist_len < 10 {
        return 0; // UNK — warming up
    }
    // floor=ceiling=non-zero: price locked at one cross level for hist_len candles → early trend
    if tf.floor == tf.ceiling && tf.floor != 0 {
        return if tf.floor > 0 { 2 } else { -2 };
    }
    // floor >= -1: never dips below DownInner → floor is high
    // ceiling <= 1: never pushes above UpInner → ceiling is low
    match (tf.floor >= -1, tf.ceiling <= 1) {
        (true, false) => 2,  // UP: high floor + pushes up
        (false, true) => -2, // DOWN: low ceiling + sinks down
        _ => 1,              // WIN: ranging (both sides active, or tight around ML)
    }
}

fn compute_conf(tf: &TfMovState) -> u8 {
    if tf.hist_len < 5 {
        return 0;
    }
    let hist_score = (tf.hist_len.min(20) as f64 / 20.0) * 40.0;
    let mov_score = tf.mov.abs().min(100.0) / 100.0 * 40.0;
    let stability = if tf.floor_age > 5 && tf.ceiling_age > 5 {
        20.0
    } else if tf.floor_age > 3 || tf.ceiling_age > 3 {
        10.0
    } else {
        0.0
    };
    ((hist_score + mov_score + stability) as u8).min(100)
}

fn fc_history_debug(history: &[crate::state::FcEvent]) -> String {
    if history.is_empty() {
        return "fc_hist=[]".to_string();
    }
    let entries: Vec<String> = history
        .iter()
        .map(|e| {
            let d = if e.dir > 0 { "^" } else { "v" };
            if e.pct >= 0.01 {
                format!("{}{:.1}%{}t", d, e.pct, e.dur)
            } else {
                format!("{}{}t", d, e.dur)
            }
        })
        .collect();
    format!("fc_hist=[{}]", entries.join(" "))
}

fn fast_candle_debug(fc: &Option<lte_strategy_bridge::SuggestionInfoCandle>) -> String {
    match fc {
        None => "fc=.".to_string(),
        Some(c) => {
            let dir = if c.exit >= c.enter { "+" } else { "-" };
            let pct = (c.exit - c.enter).abs() / c.enter.max(f64::EPSILON) * 100.0;
            let range = (c.max - c.min) / c.enter.max(f64::EPSILON) * 100.0;
            let net_qty = c.qty - c.qtym; // net USDT (positive = net buy)
            format!(
                "fc={}{:.2}%(rng={:.2}% net_qty={:.2})",
                dir, pct, range, net_qty
            )
        }
    }
}

fn vt_debug(v_lines: &[crate::state::VLineTracker; 9]) -> String {
    // Show 6 key lines: UB(8) US(6) UI(5) ML(4) DI(3) DB(0)
    let lines = [
        ("UB", &v_lines[8]),
        ("US", &v_lines[6]),
        ("UI", &v_lines[5]),
        ("ML", &v_lines[4]),
        ("DI", &v_lines[3]),
        ("DB", &v_lines[0]),
    ];
    lines
        .iter()
        .map(|(name, vl)| {
            let side = match vl.was_above {
                Some(true) => "^",
                Some(false) => "v",
                None => "-",
            };
            format!(
                "{}{}L{}({:.0}%{})S{}({:.0}%{})",
                name,
                side,
                vl.long_side.count,
                vl.long_side.last_power,
                if vl.long_side.exhausted { "!" } else { "" },
                vl.short_side.count,
                vl.short_side.last_power,
                if vl.short_side.exhausted { "!" } else { "" },
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// v0.57: runtime debug builder. Given a bitmask `sel`, returns the FULL-detail topic blocks for
// the selected presets, joined into one suffix. Driven by the persistent `debug_info` config —
// appended to every event line (init/ind/sug) so the API snapshot always carries the chosen view,
// readable straight from `list_bots` last_debug. Switching is a config push applied on the next
// tick (context accumulation), so a focused look is two switches: set preset → read → set 0.
// Reads only `state`, so it is identical on any tick/event — also ready to back a one-shot
// request action if/when a request→response channel to the bot exists. Empty when `sel == 0`.
//   1 V · 2 MRC · 4 BIG · 8 CAS · 16 DMP  (add to combine; 31 = ALL)
fn debug_builder(state: &State, sel: i64) -> String {
    if sel == 0 {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    if sel & 1 != 0 {
        // V-tracker — same compact per-line table the v-tracker skill parses (`vt=` token).
        parts.push(format!("vt={}", vt_debug(&state.v_lines)));
    }
    if sel & 2 != 0 {
        // 1m MRC band price map (low→high) + 5m mean-line.
        parts.push(format!(
            "mrc=db{:.2} do{:.2} ds{:.2} di{:.2} ml{:.2} ui{:.2} us{:.2} uo{:.2} ub{:.2} | 5ml{:.2}",
            state.mrc_db, state.mrc_do, state.mrc_ds, state.mrc_di, state.mrc_ml,
            state.mrc_ui, state.mrc_us, state.mrc_uo, state.mrc_ub, state.mrc5_ml,
        ));
    }
    if sel & 4 != 0 {
        // Big-line first-reaction bracket + stuck-side exits.
        parts.push(format!("big=l{}", state.long_big_tested as u8,));
    }
    if sel & 8 != 0 {
        // Movement cascade / current run leg / stair-cross ladder / 5m hard-break.
        parts.push(format!(
            "cas=dir{}/run{}t lc{} b5={}",
            state.mv_cas_dir, state.run_ticks, state.long_stair_cross, state.mrc5_break,
        ));
    }
    if sel & 16 != 0 {
        parts.push(format!(
            "dmp={:.2} dd={:.2}",
            uncertainty_damp(state),
            state.risk_drawdown
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" | sw[{}]", parts.join(" "))
    }
}

fn tf_debug(tf: &TfMovState) -> String {
    let r = match tf.regime {
        2 => "TUP",
        -2 => "TDN",
        1 => "SWY",
        _ => "UNK",
    };
    format!(
        "{}(m={:.0} c={} f={}/{} C={}/{} n={})",
        r, tf.mov, tf.cross, tf.floor, tf.floor_age, tf.ceiling, tf.ceiling_age, tf.candles
    )
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


// ── Indicator access ────────────────────────────────────────────────────────

type Indicators = BTreeMap<i64, Vec<HashMap<String, HashMap<String, ModuleIndicatorValue>>>>;

fn read_float(ind: &Indicators, tf: i64, idx: usize, name: &str, field: &str) -> Option<f64> {
    ind.get(&tf)?
        .get(idx)?
        .get(name)?
        .get(field)
        .and_then(|v| match v {
            ModuleIndicatorValue::Float(f) => Some(*f),
            _ => None,
        })
}

fn read_direction(
    ind: &Indicators,
    tf: i64,
    idx: usize,
    name: &str,
    field: &str,
) -> Option<Direction> {
    ind.get(&tf)?
        .get(idx)?
        .get(name)?
        .get(field)
        .and_then(|v| match v {
            ModuleIndicatorValue::Direction(d) => Some(*d),
            // ModuleIndicatorValue is #[serde(untagged)] with String before Direction,
            // so "LONG"/"SHORT" strings get deserialized as String, not Direction.
            ModuleIndicatorValue::String(s) => match s.as_str() {
                "LONG" => Some(Direction::Long),
                "SHORT" => Some(Direction::Short),
                "BOTH" => Some(Direction::Both),
                _ => None,
            },
            _ => None,
        })
}

fn read_mrc(ind: &Indicators) -> Option<[f64; 9]> {
    read_mrc_at(ind, 0)
}

// Read the 9 MRC bands at a given history offset (0 = latest). Used by the cold-start seeder to
// replay each historical candle against the bands as they were at that candle.
fn read_mrc_at(ind: &Indicators, offset: usize) -> Option<[f64; 9]> {
    Some([
        read_float(ind, TF, offset, MRC, "DownBig")?,
        read_float(ind, TF, offset, MRC, "DownOuter")?,
        read_float(ind, TF, offset, MRC, "DownSmall")?,
        read_float(ind, TF, offset, MRC, "DownInner")?,
        read_float(ind, TF, offset, MRC, "MeanLine")?,
        read_float(ind, TF, offset, MRC, "UpInner")?,
        read_float(ind, TF, offset, MRC, "UpSmall")?,
        read_float(ind, TF, offset, MRC, "UpOuter")?,
        read_float(ind, TF, offset, MRC, "UpBig")?,
    ])
}

fn read_analysis(
    input: &ModuleInput,
) -> (
    i64,
    f64,
    f64,
    Option<lte_strategy_bridge::SuggestionInfoCandle>,
) {
    match &input.sug_info {
        Some(s) => (
            s.indicator.ntps,
            s.indicator.trandm,
            s.indicator.asset_01,
            s.fast_candle.clone(),
        ),
        None => (0, 0.0, 0.0, None), // trandm default neutral (0) for the -100..+100 scale
    }
}
