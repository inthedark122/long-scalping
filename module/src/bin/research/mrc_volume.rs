use long_scalping::detection::{mrc, vlines};
use long_scalping::observer::{VolumeGainTracker, VOLUME_GAIN_THRESHOLD};
use long_scalping::state::{MrcCrossingState, VLineState};

use super::data::CandleRow;

#[derive(Debug, Clone, Copy)]
pub(crate) struct MrcVolumeEvent {
    pub(crate) time_ms: i64,
    pub(crate) price: f64,
    pub(crate) direction: i8,
    pub(crate) trade_side: i8,
    pub(crate) line: f64,
    pub(crate) target: f64,
    pub(crate) gain: f64,
    pub(crate) buy_sell_gain: f64,
    pub(crate) action: &'static str,
    pub(crate) pullback_ready: bool,
    pub(crate) no_pullback_count: u32,
    pub(crate) reverted_line: Option<u8>,
    pub(crate) crossed_line: Option<u8>,
    pub(crate) previous_direction: i8,
    pub(crate) sequence: [u8; 8],
    pub(crate) sequence_len: usize,
    pub(crate) sequence_times_ms: [u64; 8],
    pub(crate) pullback_detected: bool,
    pub(crate) crossing_kind: &'static str,
    pub(crate) entry_allowed: bool,
    pub(crate) entry_skip_reason: Option<&'static str>,
    pub(crate) risk_pct: Option<f64>,
    pub(crate) virtual_target: Option<f64>,
    pub(crate) virtual_stop: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MrcVolumeConfig {
    pub(crate) revert: bool,
    pub(crate) allow_confirm: bool,
    pub(crate) trade_side_filter: Option<i8>,
    pub(crate) max_stop_distance_pct: Option<f64>,
    pub(crate) parallel_positions: bool,
}

impl Default for MrcVolumeConfig {
    fn default() -> Self {
        Self {
            revert: false,
            allow_confirm: true,
            trade_side_filter: None,
            max_stop_distance_pct: None,
            parallel_positions: false,
        }
    }
}

impl MrcVolumeConfig {
    pub(crate) fn new(revert: bool) -> Self {
        Self {
            revert,
            allow_confirm: true,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VirtualPosition {
    pub(crate) side: i8,
    pub(crate) entry: f64,
    pub(crate) target: f64,
    pub(crate) stop: f64,
    pub(crate) opened_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct CompletedTrade {
    pub(crate) side: i8,
    pub(crate) entry_price: f64,
    pub(crate) exit_price: f64,
    pub(crate) pnl_pct: f64,
    pub(crate) opened_at: i64,
    pub(crate) closed_at: i64,
    pub(crate) exit_reason: &'static str,
}

#[derive(Debug, Default)]
pub(crate) struct MrcVolumeState {
    pub(crate) vlines: [VLineState; 9],
    pub(crate) crossing: MrcCrossingState,
    pub(crate) volume: VolumeGainTracker,
    pub(crate) tick: u64,
    pub(crate) position: Option<VirtualPosition>,
    pub(crate) parallel_positions: Vec<VirtualPosition>,
    pub(crate) completed_trades: Vec<CompletedTrade>,
    pub(crate) config: MrcVolumeConfig,
}

impl MrcVolumeState {
    pub(crate) fn new(revert: bool) -> Self {
        Self::with_config(MrcVolumeConfig::new(revert))
    }

    pub(crate) fn with_config(config: MrcVolumeConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    pub(crate) fn observe(&mut self, row: &CandleRow) -> Option<MrcVolumeEvent> {
        self.tick = self.tick.saturating_add(1);
        let previous_direction = self.crossing.movement_direction;
        if self.config.parallel_positions {
            self.close_parallel_positions(row);
        } else if let Some(position) = self.position {
            let hit_target = if position.side > 0 {
                row.high >= position.target
            } else {
                row.low <= position.target
            };
            let hit_stop = if position.side > 0 {
                row.low <= position.stop
            } else {
                row.high >= position.stop
            };
            if hit_stop {
                self.close_position(position.stop, row.time_ms, "stop");
            } else if hit_target {
                self.close_position(position.target, row.time_ms, "target");
            }
        }
        vlines::update(
            &mut self.vlines,
            &mut self.crossing,
            row.mrc.lines(),
            row.close,
            self.tick,
            row.time_ms.max(0) as u64,
        );
        let zone = mrc::analyze(&row.mrc, row.close).zone;
        self.volume.observe(
            self.tick,
            row.gain,
            row.buy_sell_gain,
            zone,
            self.crossing.candle_cross_mask,
            row.high,
            row.low,
            row.close,
        );
        self.crossing.candle_cross_mask = 0;

        let crossed = self.crossing.last_line.is_some()
            && self
                .crossing
                .last_line
                .is_some_and(|line| self.vlines[line as usize].last_cross_tick == self.tick);
        if !crossed || self.crossing.last_direction == 0 {
            return None;
        }

        let direction = self.crossing.last_direction;
        let reverted_line = if direction > 0 {
            self.crossing.reverted_long_line
        } else {
            self.crossing.reverted_short_line
        };
        let pullback_ready = self.crossing.no_pullback_count == 0 && reverted_line.is_some();
        let sequence_len = self.crossing.sequence_len;
        let crossed_line = self.crossing.last_line;
        let pullback_detected = sequence_len >= 3
            && crossed_line.is_some_and(|line| self.crossing.sequence[sequence_len - 3] == line);
        let crossing_kind = if pullback_detected {
            "pullback"
        } else if sequence_len <= 1 {
            "initial-cross"
        } else if previous_direction != 0 && previous_direction != direction {
            "counter-trend"
        } else {
            "continuation"
        };
        let signed_volume = row.buy_sell_gain.signum() as i8;
        let action = if !pullback_ready {
            "wait-no-pullback"
        } else if row.gain > VOLUME_GAIN_THRESHOLD
            && signed_volume != 0
            && signed_volume != direction
        {
            "revert"
        } else if row.gain > VOLUME_GAIN_THRESHOLD && signed_volume == direction {
            "confirm"
        } else {
            "continue"
        };
        let event = MrcVolumeEvent {
            time_ms: row.time_ms,
            price: row.close,
            direction,
            trade_side: if self.config.revert {
                -direction
            } else {
                direction
            },
            line: self.crossing.movement_start,
            target: self.crossing.movement_target,
            gain: row.gain,
            buy_sell_gain: row.buy_sell_gain,
            action,
            pullback_ready,
            no_pullback_count: self.crossing.no_pullback_count,
            reverted_line,
            crossed_line,
            previous_direction,
            sequence: self.crossing.sequence,
            sequence_len,
            sequence_times_ms: self.crossing.sequence_times_ms,
            pullback_detected,
            crossing_kind,
            entry_allowed: false,
            entry_skip_reason: None,
            risk_pct: None,
            virtual_target: None,
            virtual_stop: None,
        };
        let decision = self.update_virtual_position(row, event);
        Some(MrcVolumeEvent {
            entry_allowed: decision.allowed,
            entry_skip_reason: decision.skip_reason,
            risk_pct: decision.risk_pct,
            virtual_target: decision.virtual_target,
            virtual_stop: decision.virtual_stop,
            ..event
        })
    }

    pub(crate) fn open_position_count(&self) -> usize {
        if self.config.parallel_positions {
            self.parallel_positions.len()
        } else {
            usize::from(self.position.is_some())
        }
    }

    pub(crate) fn has_position_at(&self, opened_at: i64) -> bool {
        if self.config.parallel_positions {
            self.parallel_positions
                .iter()
                .any(|position| position.opened_at == opened_at)
        } else {
            self.position
                .is_some_and(|position| position.opened_at == opened_at)
        }
    }

    pub(crate) fn clear_position_at(&mut self, opened_at: i64) {
        if self.config.parallel_positions {
            self.parallel_positions
                .retain(|position| position.opened_at != opened_at);
        } else if self
            .position
            .is_some_and(|position| position.opened_at == opened_at)
        {
            self.position = None;
        }
    }

    pub(crate) fn clear_position(&mut self) {
        self.position = None;
        self.parallel_positions.clear();
    }

    pub(crate) fn finish(&mut self, row: &CandleRow) {
        if self.config.parallel_positions {
            self.close_all_parallel(row.close, row.time_ms, "end-of-history");
        } else {
            self.close_position(row.close, row.time_ms, "end-of-history");
        }
    }

    fn close_position(&mut self, exit_price: f64, closed_at: i64, exit_reason: &'static str) {
        let Some(position) = self.position.take() else {
            return;
        };
        self.record_trade(position, exit_price, closed_at, exit_reason);
    }

    fn record_trade(
        &mut self,
        position: VirtualPosition,
        exit_price: f64,
        closed_at: i64,
        exit_reason: &'static str,
    ) {
        let pnl_pct = if position.side > 0 {
            (exit_price - position.entry) / position.entry * 100.0
        } else {
            (position.entry - exit_price) / position.entry * 100.0
        };
        self.completed_trades.push(CompletedTrade {
            side: position.side,
            entry_price: position.entry,
            exit_price,
            pnl_pct,
            opened_at: position.opened_at,
            closed_at,
            exit_reason,
        });
    }

    fn close_parallel_positions(&mut self, row: &CandleRow) {
        let positions = std::mem::take(&mut self.parallel_positions);
        let mut remaining = Vec::with_capacity(positions.len());
        for position in positions {
            let hit_target = if position.side > 0 {
                row.high >= position.target
            } else {
                row.low <= position.target
            };
            let hit_stop = if position.side > 0 {
                row.low <= position.stop
            } else {
                row.high >= position.stop
            };
            if hit_stop {
                self.record_trade(position, position.stop, row.time_ms, "stop");
            } else if hit_target {
                self.record_trade(position, position.target, row.time_ms, "target");
            } else {
                remaining.push(position);
            }
        }
        self.parallel_positions = remaining;
    }

    fn close_all_parallel(&mut self, exit_price: f64, closed_at: i64, exit_reason: &'static str) {
        let positions = std::mem::take(&mut self.parallel_positions);
        for position in positions {
            self.record_trade(position, exit_price, closed_at, exit_reason);
        }
    }

    fn update_virtual_position(&mut self, row: &CandleRow, event: MrcVolumeEvent) -> EntryDecision {
        if event.action == "revert" {
            if self.config.parallel_positions {
                self.close_all_parallel(row.close, row.time_ms, "volume-revert");
            } else {
                self.close_position(row.close, row.time_ms, "volume-revert");
            }
            return EntryDecision::skipped("volume-revert");
        }
        if event.action == "wait-no-pullback" {
            return EntryDecision::skipped("no-pullback");
        }
        if event.target <= 0.0 {
            return EntryDecision::skipped("invalid-target");
        }
        if !self.config.allow_confirm && event.action == "confirm" {
            return EntryDecision::skipped("confirm-disabled");
        }
        if !self.config.parallel_positions && self.position.is_some() {
            return EntryDecision::skipped("position-active");
        }
        let distance = (event.target - event.line).abs();
        if distance <= 0.0 {
            return EntryDecision::skipped("zero-distance");
        }
        let normal_stop = if event.direction > 0 {
            event.line - distance * 0.8
        } else {
            event.line + distance * 0.8
        };
        let (side, target, stop) = if self.config.revert {
            (-event.direction, normal_stop, event.target)
        } else {
            (event.direction, event.target, normal_stop)
        };
        let risk_pct = (stop - row.close).abs() / row.close * 100.0;
        if let Some(max_stop_distance_pct) = self.config.max_stop_distance_pct {
            if risk_pct > max_stop_distance_pct {
                return EntryDecision {
                    risk_pct: Some(risk_pct),
                    virtual_target: Some(target),
                    virtual_stop: Some(stop),
                    ..EntryDecision::skipped("risk-cap")
                };
            }
        }
        if self
            .config
            .trade_side_filter
            .is_some_and(|allowed_side| allowed_side != side)
        {
            return EntryDecision {
                risk_pct: Some(risk_pct),
                virtual_target: Some(target),
                virtual_stop: Some(stop),
                ..EntryDecision::skipped("side-filter")
            };
        }
        let position = VirtualPosition {
            side,
            entry: row.close,
            target,
            stop,
            opened_at: row.time_ms,
        };
        if self.config.parallel_positions {
            self.parallel_positions.push(position);
        } else {
            self.position = Some(position);
        }
        EntryDecision {
            allowed: true,
            skip_reason: None,
            risk_pct: Some(risk_pct),
            virtual_target: Some(target),
            virtual_stop: Some(stop),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EntryDecision {
    allowed: bool,
    skip_reason: Option<&'static str>,
    risk_pct: Option<f64>,
    virtual_target: Option<f64>,
    virtual_stop: Option<f64>,
}

impl EntryDecision {
    fn skipped(reason: &'static str) -> Self {
        Self {
            allowed: false,
            skip_reason: Some(reason),
            risk_pct: None,
            virtual_target: None,
            virtual_stop: None,
        }
    }
}

pub(crate) fn summarize_trades(trades: &[CompletedTrade]) -> String {
    if trades.is_empty() {
        return "trades=0\nno completed trades".to_string();
    }
    let wins = trades.iter().filter(|trade| trade.pnl_pct > 0.0).count();
    let losses = trades.len() - wins;
    let total_pnl = trades.iter().map(|trade| trade.pnl_pct).sum::<f64>();
    let average_pnl = total_pnl / trades.len() as f64;
    let average_win = if wins == 0 {
        0.0
    } else {
        trades
            .iter()
            .filter(|trade| trade.pnl_pct > 0.0)
            .map(|trade| trade.pnl_pct)
            .sum::<f64>()
            / wins as f64
    };
    let average_loss = if losses == 0 {
        0.0
    } else {
        trades
            .iter()
            .filter(|trade| trade.pnl_pct <= 0.0)
            .map(|trade| trade.pnl_pct)
            .sum::<f64>()
            / losses as f64
    };
    format!(
        "trades={}\nwins={}\nlosses={}\nwin_rate={:.1}%\ntotal_pnl={:.4}%\navg_pnl={:.4}%\navg_win={:.4}%\navg_loss={:.4}%",
        trades.len(),
        wins,
        losses,
        wins as f64 / trades.len() as f64 * 100.0,
        total_pnl,
        average_pnl,
        average_win,
        average_loss,
    )
}
