#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    SlowRise,
    SlowFall,
    UpStaircase,
    DownStaircase,
    MeanRange,
}

impl PatternKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SlowRise => "SLOW_RISE",
            Self::SlowFall => "SLOW_FALL",
            Self::UpStaircase => "STAIRCASE_UP",
            Self::DownStaircase => "STAIRCASE_DOWN",
            Self::MeanRange => "MEAN_RANGE",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::SlowRise => "Most closes rise gradually across the 30 minute window.",
            Self::SlowFall => "Most closes fall gradually across the 30 minute window.",
            Self::UpStaircase => "Confirmed swing highs and swing lows both advance.",
            Self::DownStaircase => "Confirmed swing highs and swing lows both decline.",
            Self::MeanRange => "Price repeatedly crosses its EMA while staying range bound.",
        }
    }

    pub const fn direction(self) -> i8 {
        match self {
            Self::SlowRise | Self::UpStaircase => 1,
            Self::SlowFall | Self::DownStaircase => -1,
            Self::MeanRange => 0,
        }
    }
}

pub const DEFINITIONS: [PatternKind; 5] = [
    PatternKind::SlowRise,
    PatternKind::SlowFall,
    PatternKind::UpStaircase,
    PatternKind::DownStaircase,
    PatternKind::MeanRange,
];
