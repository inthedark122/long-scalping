use std::collections::HashMap;

use crate::debug::{
    cascade_debug, debug_builder, fast_candle_debug, fc_history_debug, tf_debug, vt_debug,
};
use crate::fastcandle::update_fast_candles;
use crate::indicators::*;
use crate::movement::{
    advance_market_mode, advance_tf_from_ind, init_cascade, init_tf_from_ind, update_cascade,
    update_risk_ratchet,
};
use crate::orders::{close, market_open, tp_order_with_band};
use crate::sizing::{
    compute_auto_open, compute_limit_orders, fcscalp_support, risk_sized_notional, uncertainty_damp,
    upnl,
};
use crate::vlines::{seed_v_lines, update_v_lines};
use crate::state::State;
use crate::window::{detect_window, window_debug};
use lte_strategy_bridge::abi::{
    Direction, ModuleClosePosition, ModuleEvent, ModuleInput, ModuleOpenPosition, ModuleOutput,
    ModuleUpdatePosition, OrderRole,
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

    // Rebuild both movement cascades from last 10 candles and initialise risk ratchet.
    init_cascade(&mut state.cas1, &input.indicators, TF);
    init_cascade(&mut state.cas5, &input.indicators, TF_5M);
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
            state.mkt_mode, state.cas1.cas_dir, state.cas1.avg_size,
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
        update_cascade(&mut state.cas1, state.vol_bsg, c_high, c_low);
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
            // Feed the 5m cascade on each 5m close (observe-only): larger blocks for the
            // bigger move magnitude / window / direction context.
            let bsg5 =
                read_float(&input.indicators, TF_5M, 0, "volume_50", "BuySellGain").unwrap_or(0.0);
            update_cascade(&mut state.cas5, bsg5, hi5, lo5);
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
        && (state.cfg.enable_stairs || state.cfg.enable_trend || state.cfg.enable_fcscalp)
    {
        // Flat → clear stale owner before this candle's open re-latches (mirror of
        // the reconciliation in handle_sug_info; the two handlers can each open).
        if input.positions.long.is_none() {
            state.pos_owner = OWN_NONE;
        }
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
            // Latch ownership here too — opens fire from BOTH this candle handler and
            // handle_sug_info; without this an FC open on a candle close stays OWN_NONE
            // and the base TP/DCA logic wrongly manages the FC position.
            if state.pos_owner == OWN_NONE {
                state.pos_owner = owner_from_mark(&op.note);
            }
            // Record the FC limit open's price so it can be trailed (not re-emitted).
            if op.note == "fcs-l" {
                if let Some(px) = op.enter_price {
                    state.fc_open_px = px;
                }
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
            state.mkt_mode, state.cas1.cas_dir,
            state.cas1.cas_high - state.cas1.cas_low,
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
    // Owner reconciliation: when genuinely flat, clear the owner so a stale latch from an
    // unfilled/rejected open can't leak onto the next position. The open this tick (emitted
    // while still flat) re-latches below; once has_long is true the owner is preserved.
    if !has_long {
        state.pos_owner = OWN_NONE;
    }

    // FC grid rung tracking: a buy fill (open or grid rung) increases position qty — qty is
    // price-independent, so an increase is an unambiguous fill. Advance the spacing anchor
    // (last fill price) and rung count so compute_limit_orders places the NEXT rung only
    // after this one fills, and at least fcscalp_rung_gap below it (sequential + spaced).
    let long_qty = input.positions.long.as_ref().map(|p| p.qty).unwrap_or(0.0);
    if !has_long {
        state.fc_rungs = 0;
        state.fc_last_fill_px = 0.0;
        state.fc_prev_qty = 0.0;
    } else {
        if long_qty > state.fc_prev_qty + 1e-9 {
            state.fc_rungs = state.fc_rungs.saturating_add(1);
            state.fc_last_fill_px = price;
        }
        state.fc_prev_qty = long_qty; // track adds (up) and TP reductions (down)
        state.fc_open_px = 0.0; // open filled into a position — no pending open to trail
    }
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

    // ── Exit routing by owner (docs/strategy-arbitration.md §2) ─────────────────
    // Only the strategy that OPENED the position manages it. fc_owner → FC's deep stop
    // only; base_owner (AUTO/STAIR/TREND/TRANDM/REENTRY) → the base loss-exits. The
    // catastrophic margin SL still backstops every owner regardless.
    let fc_owner = state.pos_owner == OWN_FC;
    let base_owner = (OWN_AUTO..=OWN_REENTRY).contains(&state.pos_owner);

    // ── Fast-candle scalp stop: market close on a DownBig breach ────────────────
    // The entry + TP are maker limits; only this rare protective stop is a market.
    if fc_owner && has_long && long_n > 1.0 && price <= state.mrc_db {
        close_positions.push(close(Direction::Long, "fcs-sl"));
    }

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

    // ── Trend mode zone-based SL ─────────────────────────────────────────
    // Fires when price reverses one cross zone past the entry zone.
    // Tighter than the margin SL — cuts the loss early while the thesis is wrong.
    if base_owner
        && state.trend_long_sl_cross != 127
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
    if sudden_move && base_owner {
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
            && base_owner
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
        && (state.cfg.enable_stairs || state.cfg.enable_trend || state.cfg.enable_fcscalp)
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
            // Latch ownership on the flat→open transition: the first opening note this
            // tick decides which strategy owns the position; exits then route per owner.
            if state.pos_owner == OWN_NONE {
                state.pos_owner = owner_from_mark(&op.note);
            }
            // Record the FC limit open's price so it can be trailed (not re-emitted).
            if op.note == "fcs-l" {
                if let Some(px) = op.enter_price {
                    state.fc_open_px = px;
                }
            }
        }
        ops
    } else {
        Vec::new()
    };

    // ── FC entry trail / stale-cancel (ModuleUpdatePosition) ────────────────────
    // The FC open is a pending maker limit. While the dip-buy setup is still fresh,
    // amend its enter_price in place to trail it DOWN to the nearest support (better
    // fill, never chase up). Once the setup goes stale (price pumped away / flush
    // expired), close the pending order so it can't fill later on an unrelated dip.
    let fc_flush_fresh = state.fc.last_end_dir == -1
        && state.fc.last_end_tick > 0
        && state.ticks.saturating_sub(state.fc.last_end_tick) <= 3 * TICKS_PER_CANDLE;
    let fc_armable = state.cfg.enable_fcscalp && state.tf5.regime == 1 && fc_flush_fresh;
    let mut update_positions: Vec<ModuleUpdatePosition> = Vec::new();
    if !has_long && state.fc_open_px > 0.0 {
        if fc_armable {
            if let Some(support) = fcscalp_support(input, price) {
                if support < state.fc_open_px && support < price {
                    update_positions.push(ModuleUpdatePosition {
                        direction: Direction::Long,
                        enter_price: Some(support),
                        take_profit: None,
                        stop_loss: None,
                    });
                    state.fc_open_px = support;
                }
            }
        } else {
            close_positions.push(close(Direction::Long, "fcs-stale"));
            state.fc_open_px = 0.0;
        }
    }

    let tick = input.symbol_info.price_tick_size.max(f64::EPSILON);

    let mut place_orders = if state.has_mrc {
        compute_limit_orders(input, state)
    } else {
        vec![]
    };

    // ── Trend mode: gradual transition close for a long caught in a confirmed downtrend ────
    // Stage 0→1: 25% after 2 candles · 1→2: 25% after 5 · 2→3: 50% after 8.
    let mc = state.mkt_mode_candles;
    if base_owner && mkt_mode == -2 && has_long && long_n > 1.0 {
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
        // base owners only — FC holds through the flush on its deep stop, no staged SL.
        if base_owner && has_long && long_pnl < 0.0 && long_n > 1.0 {
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

    // Window context (Layer 1, observe-only): is it safe to scalp + the bounds/scale.
    // Computed and surfaced in the debug; FC does not consume it yet.
    let win = detect_window(input, state);

    // ── FC grid cancel-on-stale ─────────────────────────────────────────────────
    // Standing limit orders persist by mark until cancelled (they are NOT auto-pulled
    // when we stop emitting them). When FLAT and the FC setup is stale, cancel any
    // leftover grid rungs (fcs-g, real place-orders) so they can't fill on an unrelated
    // dip. The pending OPEN (fcs-l) is handled above via trail / close, not here.
    let mut cancel_orders: Vec<String> = Vec::new();
    if !has_long && !fc_armable {
        cancel_orders.push("fcs-g".to_string());
        // belt-and-suspenders for the pending open (also closed via close_positions above) —
        // whichever path the platform honors for a pending-limit open.
        cancel_orders.push("fcs-l".to_string());
    }

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
        cancel_orders,
        update_positions,
        debug: format!(
            "sug t={} p={:.2} ntps={} avg={:.0} x{:.1} mode={} mv={}/{:.0}(a={:.0}) brk={} drf={:.2} | ln={:.1} pnl={:.4} sl={:.2} fm={:.3} ord={} | cas1=[{}] cas5=[{}] win=[{}] |{} | {} | vt={}{}",
            state.ticks, price, ntps, state.ntps_avg, state.ntps_mult,
            state.mkt_mode, state.cas1.cas_dir,
            state.cas1.cas_high - state.cas1.cas_low, state.cas1.avg_size,
            state.cas1.break_n, state.risk_drawdown,
            long_n, total_pnl, -(margin * state.cfg.sl_margin_pct),
            margin - long_n / input.leverage.max(1) as f64 + total_pnl,
            ord_count,
            cascade_debug(&state.cas1, price),
            cascade_debug(&state.cas5, price),
            window_debug(&win),
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
        state.pos_owner = OWN_NONE; // position flat → clear owner; next open re-latches.
    }
    ModuleOutput {
        debug: format!(
            "finish {:?} t={} p={:.2}",
            direction, state.ticks, input.price
        ),
        ..Default::default()
    }
}

