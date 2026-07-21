use crate::model::TradeMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategyPolicy {
    pub(super) trade_mode: TradeMode,
    pub(super) entry_wait_gate: bool,
    pub(super) breakeven_reclaim: bool,
    pub(super) virtual_sl_tp_advance: bool,
}

impl StrategyPolicy {
    pub fn for_mode(trade_mode: TradeMode) -> Self {
        match trade_mode {
            TradeMode::Single => Self {
                trade_mode,
                entry_wait_gate: true,
                breakeven_reclaim: false,
                virtual_sl_tp_advance: false,
            },
            TradeMode::Martingale => Self {
                trade_mode,
                entry_wait_gate: false,
                breakeven_reclaim: false,
                virtual_sl_tp_advance: true,
            },
        }
    }
}
