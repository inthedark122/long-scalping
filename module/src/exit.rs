use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;
use crate::model::PositionState;
use crate::orders::tp_mark;
use lte_strategy_bridge::abi::Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    NoPosition,
    RulesNotImplemented,
}

impl Decision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoPosition => "no-position",
            Self::RulesNotImplemented => "rules-not-implemented",
        }
    }
}

pub fn evaluate(position: PositionState, _analysis: Option<&MrcCrossAnalysis>) -> Decision {
    if position == PositionState::Flat {
        Decision::NoPosition
    } else {
        Decision::RulesNotImplemented
    }
}

pub(crate) fn all_tp_marks(direction: Direction) -> Vec<String> {
    let base = tp_mark(direction);
    std::iter::once(base.clone())
        .chain(std::iter::once(format!("{base}-far")))
        .chain((1..=3).map(move |index| format!("{base}-{index}")))
        .collect()
}
