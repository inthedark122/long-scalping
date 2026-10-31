use crate::detection::mrc_cross_analysis::MrcCrossAnalysis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    RulesNotImplemented,
}

impl Decision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RulesNotImplemented => "rules-not-implemented",
        }
    }
}

pub fn evaluate(_analysis: Option<&MrcCrossAnalysis>) -> Decision {
    Decision::RulesNotImplemented
}
