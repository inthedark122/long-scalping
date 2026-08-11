#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategyPolicy {
    pub(super) entry_wait_gate: bool,
    pub(super) breakeven_reclaim: bool,
}

impl StrategyPolicy {
    pub fn single() -> Self {
        Self {
            entry_wait_gate: true,
            breakeven_reclaim: false,
        }
    }
}
