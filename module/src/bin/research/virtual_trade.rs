use serde::Serialize;

use super::data::CandleRow;

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct FeeModel {
    pub(crate) limit_fee_pct: f64,
    pub(crate) market_fee_pct: f64,
}

impl Default for FeeModel {
    fn default() -> Self {
        Self {
            limit_fee_pct: 0.02,
            market_fee_pct: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VirtualPosition {
    pub(crate) side: i8,
    pub(crate) source_line: usize,
    pub(crate) source_name: &'static str,
    pub(crate) target_line: usize,
    pub(crate) target_name: &'static str,
    pub(crate) entry_price: f64,
    pub(crate) target_price: f64,
    pub(crate) stop_price: f64,
    pub(crate) opened_at: i64,
    pub(crate) reason: String,
    pub(crate) max_favorable_price: f64,
    pub(crate) max_adverse_price: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VirtualTradeResult {
    pub(crate) side: i8,
    pub(crate) source_line: usize,
    pub(crate) source_name: &'static str,
    pub(crate) target_line: usize,
    pub(crate) target_name: &'static str,
    pub(crate) entry_price: f64,
    pub(crate) target_price: f64,
    pub(crate) stop_price: f64,
    pub(crate) exit_price: f64,
    pub(crate) gross_pnl_pct: f64,
    pub(crate) pnl_pct: f64,
    pub(crate) entry_fee_pct: f64,
    pub(crate) exit_fee_pct: f64,
    pub(crate) exit_order_type: String,
    pub(crate) mfe_pct: f64,
    pub(crate) mae_pct: f64,
    pub(crate) opened_at: i64,
    pub(crate) closed_at: i64,
    pub(crate) exit_reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct VirtualTradeSummary {
    pub(crate) trades: usize,
    pub(crate) wins: usize,
    pub(crate) losses: usize,
    pub(crate) win_rate_pct: f64,
    pub(crate) total_pnl_pct: f64,
    pub(crate) total_gross_pnl_pct: f64,
    pub(crate) total_fees_pct: f64,
    pub(crate) max_drawdown_pct: f64,
    pub(crate) average_pnl_pct: f64,
    pub(crate) average_win_pnl_pct: f64,
    pub(crate) average_loss_pnl_pct: f64,
    pub(crate) average_mfe_pct: f64,
    pub(crate) average_mae_pct: f64,
    pub(crate) average_duration_ms: Option<f64>,
}

impl VirtualTradeSummary {
    pub(crate) fn from_trades(trades: &[VirtualTradeResult]) -> Self {
        if trades.is_empty() {
            return Self::default();
        }
        let wins = trades.iter().filter(|trade| trade.pnl_pct > 0.0).count();
        let total_pnl_pct = trades.iter().map(|trade| trade.pnl_pct).sum::<f64>();
        let total_gross_pnl_pct = trades.iter().map(|trade| trade.gross_pnl_pct).sum::<f64>();
        let total_fees_pct = trades
            .iter()
            .map(|trade| trade.entry_fee_pct + trade.exit_fee_pct)
            .sum::<f64>();
        let losses = trades.len() - wins;
        let average_win_pnl_pct = if wins == 0 {
            0.0
        } else {
            trades
                .iter()
                .filter(|trade| trade.pnl_pct > 0.0)
                .map(|trade| trade.pnl_pct)
                .sum::<f64>()
                / wins as f64
        };
        let average_loss_pnl_pct = if losses == 0 {
            0.0
        } else {
            trades
                .iter()
                .filter(|trade| trade.pnl_pct <= 0.0)
                .map(|trade| trade.pnl_pct)
                .sum::<f64>()
                / losses as f64
        };
        let mut equity: f64 = 0.0;
        let mut peak: f64 = 0.0;
        let mut max_drawdown_pct: f64 = 0.0;
        for trade in trades {
            equity += trade.pnl_pct;
            peak = peak.max(equity);
            max_drawdown_pct = max_drawdown_pct.max(peak - equity);
        }
        let duration_sum = trades
            .iter()
            .map(|trade| trade.closed_at.saturating_sub(trade.opened_at))
            .sum::<i64>();
        Self {
            trades: trades.len(),
            wins,
            losses,
            win_rate_pct: wins as f64 / trades.len() as f64 * 100.0,
            total_pnl_pct,
            total_gross_pnl_pct,
            total_fees_pct,
            max_drawdown_pct,
            average_pnl_pct: total_pnl_pct / trades.len() as f64,
            average_win_pnl_pct,
            average_loss_pnl_pct,
            average_mfe_pct: trades.iter().map(|trade| trade.mfe_pct).sum::<f64>()
                / trades.len() as f64,
            average_mae_pct: trades.iter().map(|trade| trade.mae_pct).sum::<f64>()
                / trades.len() as f64,
            average_duration_ms: Some(duration_sum as f64 / trades.len() as f64),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TradeSetup {
    pub(crate) side: i8,
    pub(crate) source_line: usize,
    pub(crate) source_name: &'static str,
    pub(crate) target_line: usize,
    pub(crate) target_name: &'static str,
    pub(crate) entry_price: f64,
    pub(crate) target_price: f64,
    pub(crate) stop_price: f64,
    pub(crate) opened_at: i64,
    pub(crate) reason: String,
}

impl TradeSetup {
    pub(crate) fn new(
        side: i8,
        entry_price: f64,
        target_price: f64,
        stop_price: f64,
        opened_at: i64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            side,
            source_line: 0,
            source_name: "entry",
            target_line: 0,
            target_name: "target",
            entry_price,
            target_price,
            stop_price,
            opened_at,
            reason: reason.into(),
        }
    }

    pub(crate) fn with_levels(
        side: i8,
        source_line: usize,
        source_name: &'static str,
        target_line: usize,
        target_name: &'static str,
        entry_price: f64,
        target_price: f64,
        stop_price: f64,
        opened_at: i64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            side,
            source_line,
            source_name,
            target_line,
            target_name,
            entry_price,
            target_price,
            stop_price,
            opened_at,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VirtualTradeSimulator {
    fees: FeeModel,
    active: Option<VirtualPosition>,
    completed: Vec<VirtualTradeResult>,
}

impl Default for VirtualTradeSimulator {
    fn default() -> Self {
        Self::new(FeeModel::default())
    }
}

impl VirtualTradeSimulator {
    pub(crate) fn new(fees: FeeModel) -> Self {
        Self {
            fees,
            active: None,
            completed: Vec::new(),
        }
    }

    pub(crate) fn open(&mut self, setup: TradeSetup) {
        if self.active.is_some() {
            return;
        }
        self.active = Some(VirtualPosition {
            side: setup.side,
            source_line: setup.source_line,
            source_name: setup.source_name,
            target_line: setup.target_line,
            target_name: setup.target_name,
            entry_price: setup.entry_price,
            target_price: setup.target_price,
            stop_price: setup.stop_price,
            opened_at: setup.opened_at,
            reason: setup.reason,
            max_favorable_price: setup.entry_price,
            max_adverse_price: setup.entry_price,
        });
    }

    pub(crate) fn update(&mut self, row: &CandleRow) {
        let Some((hit_stop, hit_target, stop_price, target_price)) =
            self.active.as_mut().map(|trade| {
                if trade.side > 0 {
                    trade.max_favorable_price = trade.max_favorable_price.max(row.high);
                    trade.max_adverse_price = trade.max_adverse_price.min(row.low);
                } else {
                    trade.max_favorable_price = trade.max_favorable_price.min(row.low);
                    trade.max_adverse_price = trade.max_adverse_price.max(row.high);
                }
                let hit_stop = if trade.side > 0 {
                    row.low <= trade.stop_price
                } else {
                    row.high >= trade.stop_price
                };
                let hit_target = if trade.side > 0 {
                    row.high >= trade.target_price
                } else {
                    row.low <= trade.target_price
                };
                (hit_stop, hit_target, trade.stop_price, trade.target_price)
            })
        else {
            return;
        };
        if hit_stop {
            self.close_active(stop_price, row.time_ms, "stop", "market");
        } else if hit_target {
            self.close_active(target_price, row.time_ms, "target", "limit");
        }
    }

    pub(crate) fn close_active(
        &mut self,
        exit_price: f64,
        closed_at: i64,
        exit_reason: &str,
        exit_order_type: &str,
    ) {
        let Some(trade) = self.active.take() else {
            return;
        };
        let gross_pnl_pct = if trade.side > 0 {
            (exit_price - trade.entry_price) / trade.entry_price * 100.0
        } else {
            (trade.entry_price - exit_price) / trade.entry_price * 100.0
        };
        let entry_fee_pct = self.fees.limit_fee_pct;
        let exit_fee_rate = if exit_order_type == "limit" {
            self.fees.limit_fee_pct
        } else {
            self.fees.market_fee_pct
        };
        let exit_fee_pct = exit_fee_rate * exit_price / trade.entry_price;
        let net_pnl_pct = gross_pnl_pct - entry_fee_pct - exit_fee_pct;
        let (mfe_price, mae_price) = if trade.side > 0 {
            (
                trade.max_favorable_price - trade.entry_price,
                trade.entry_price - trade.max_adverse_price,
            )
        } else {
            (
                trade.entry_price - trade.max_favorable_price,
                trade.max_adverse_price - trade.entry_price,
            )
        };
        self.completed.push(VirtualTradeResult {
            side: trade.side,
            source_line: trade.source_line,
            source_name: trade.source_name,
            target_line: trade.target_line,
            target_name: trade.target_name,
            entry_price: trade.entry_price,
            target_price: trade.target_price,
            stop_price: trade.stop_price,
            exit_price,
            gross_pnl_pct,
            pnl_pct: net_pnl_pct,
            entry_fee_pct,
            exit_fee_pct,
            exit_order_type: exit_order_type.to_string(),
            mfe_pct: mfe_price / trade.entry_price * 100.0,
            mae_pct: mae_price / trade.entry_price * 100.0,
            opened_at: trade.opened_at,
            closed_at,
            exit_reason: exit_reason.to_string(),
        });
    }

    pub(crate) fn finish(&mut self, row: &CandleRow) {
        self.update(row);
        self.close_active(row.close, row.time_ms, "end-of-history", "market");
    }

    pub(crate) fn active(&self) -> Option<VirtualPosition> {
        self.active.clone()
    }

    pub(crate) fn completed(&self) -> Vec<VirtualTradeResult> {
        self.completed.clone()
    }

    pub(crate) fn completed_count(&self) -> usize {
        self.completed.len()
    }

    pub(crate) fn last_completed(&self) -> Option<VirtualTradeResult> {
        self.completed.last().cloned()
    }

    pub(crate) fn summary(&self) -> VirtualTradeSummary {
        VirtualTradeSummary::from_trades(&self.completed)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TradeEvent {
    pub(crate) kind: &'static str,
    pub(crate) side: &'static str,
    pub(crate) time_ms: i64,
    pub(crate) reason: String,
    pub(crate) pnl_pct: f64,
}

pub(crate) fn best_fill(candle: &CandleRow, side: i8) -> f64 {
    if side > 0 {
        candle.low
    } else if side < 0 {
        candle.high
    } else {
        candle.close
    }
}

pub(crate) fn setup_from_candle(
    candle: &CandleRow,
    side: i8,
    reason: &str,
    tp_base: f64,
    tp_mult: f64,
    sl_mult: f64,
) -> Option<TradeSetup> {
    if side == 0 || tp_base <= 0.0 {
        return None;
    }
    let entry_pct = best_fill(candle, side);
    let (tp_pct, sl_pct) = if side > 0 {
        (entry_pct + tp_base * tp_mult, entry_pct - tp_base * sl_mult)
    } else {
        (entry_pct - tp_base * tp_mult, entry_pct + tp_base * sl_mult)
    };
    Some(TradeSetup::new(
        side,
        entry_pct,
        tp_pct,
        sl_pct,
        candle.time_ms,
        reason,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(time_ms: i64, high: f64, low: f64, close: f64) -> CandleRow {
        CandleRow {
            time_ms,
            open: close,
            high,
            low,
            close,
            zz_dir: 0,
            zz_break_price: 0.0,
            gain: 0.0,
            buy_sell_gain: 0.0,
            ntps: 0,
            mrc: Default::default(),
        }
    }

    #[test]
    fn market_stop_fee_and_drawdown_are_included_in_net_results() {
        let mut simulator = VirtualTradeSimulator::default();
        simulator.open(TradeSetup::with_levels(
            1, 4, "mean", 5, "up_inner", 100.0, 104.0, 96.0, 0, "test",
        ));
        simulator.update(&candle(60_000, 100.0, 95.0, 95.0));

        let trades = simulator.completed();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].exit_order_type, "market");
        assert!((trades[0].entry_fee_pct - 0.02).abs() < 1e-12);
        assert!((trades[0].exit_fee_pct - 0.048).abs() < 1e-12);
        assert!((trades[0].pnl_pct + 4.068).abs() < 1e-12);
        assert!((simulator.summary().max_drawdown_pct - 4.068).abs() < 1e-12);
    }
}
