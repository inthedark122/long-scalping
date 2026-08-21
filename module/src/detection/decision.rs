use super::mrc_pattern_classification::{MrcPatternKind, StaircaseLifecycle, StaircaseTransition};

pub const PRIMARY_STAIRS: &str = "primary-stairs";
pub const SUPPORT_FAST_BREAK_RETURN: &str = "support-fast-break-return";
pub const SUPPORT_PULLBACK: &str = "support-pullback";
pub const SUPPORT_VOLUME: &str = "support-volume";
pub const SUPPORT_EXTREME: &str = "support-extreme";
pub const SUPPORT_TREND: &str = "support-trend";

pub const DEFAULT_COMPONENTS: [&str; 6] = [
    PRIMARY_STAIRS,
    SUPPORT_FAST_BREAK_RETURN,
    SUPPORT_PULLBACK,
    SUPPORT_VOLUME,
    SUPPORT_EXTREME,
    SUPPORT_TREND,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionComponent {
    PrimaryStairs,
    SupportFastBreakReturn,
    SupportPullback,
    SupportVolume,
    SupportExtreme,
    SupportTrend,
}

impl DecisionComponent {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            PRIMARY_STAIRS => Self::PrimaryStairs,
            SUPPORT_FAST_BREAK_RETURN => Self::SupportFastBreakReturn,
            SUPPORT_PULLBACK => Self::SupportPullback,
            SUPPORT_VOLUME => Self::SupportVolume,
            SUPPORT_EXTREME => Self::SupportExtreme,
            SUPPORT_TREND => Self::SupportTrend,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::PrimaryStairs => PRIMARY_STAIRS,
            Self::SupportFastBreakReturn => SUPPORT_FAST_BREAK_RETURN,
            Self::SupportPullback => SUPPORT_PULLBACK,
            Self::SupportVolume => SUPPORT_VOLUME,
            Self::SupportExtreme => SUPPORT_EXTREME,
            Self::SupportTrend => SUPPORT_TREND,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionComponents {
    pub primary_stairs: bool,
    pub support_fast_break_return: bool,
    pub support_pullback: bool,
    pub support_volume: bool,
    pub support_extreme: bool,
    pub support_trend: bool,
}

impl Default for DecisionComponents {
    fn default() -> Self {
        Self {
            primary_stairs: true,
            support_fast_break_return: true,
            support_pullback: true,
            support_volume: true,
            support_extreme: true,
            support_trend: true,
        }
    }
}

impl DecisionComponents {
    pub fn for_primary_mode(mode: &str) -> Option<Self> {
        let components = Self::default();
        match mode.to_ascii_lowercase().as_str() {
            "all" | "both" | "stairs" | "stairs-only" | "long-stairs" => {}
            _ => return None,
        }
        Some(components)
    }

    pub fn from_names(names: &[String]) -> (Self, Vec<String>) {
        let mut components = Self {
            primary_stairs: false,
            support_fast_break_return: false,
            support_pullback: false,
            support_volume: false,
            support_extreme: false,
            support_trend: false,
        };
        let mut unknown = Vec::new();
        for name in names {
            match DecisionComponent::parse(name) {
                Some(DecisionComponent::PrimaryStairs) => components.primary_stairs = true,
                Some(DecisionComponent::SupportFastBreakReturn) => {
                    components.support_fast_break_return = true
                }
                Some(DecisionComponent::SupportPullback) => components.support_pullback = true,
                Some(DecisionComponent::SupportVolume) => components.support_volume = true,
                Some(DecisionComponent::SupportExtreme) => components.support_extreme = true,
                Some(DecisionComponent::SupportTrend) => components.support_trend = true,
                None => unknown.push(name.clone()),
            }
        }
        (components, unknown)
    }

    pub fn names(self) -> Vec<String> {
        [
            (self.primary_stairs, PRIMARY_STAIRS),
            (self.support_fast_break_return, SUPPORT_FAST_BREAK_RETURN),
            (self.support_pullback, SUPPORT_PULLBACK),
            (self.support_volume, SUPPORT_VOLUME),
            (self.support_extreme, SUPPORT_EXTREME),
            (self.support_trend, SUPPORT_TREND),
        ]
        .into_iter()
        .filter_map(|(enabled, name)| enabled.then_some(name.to_string()))
        .collect()
    }

    pub fn enabled(self, component: DecisionComponent) -> bool {
        match component {
            DecisionComponent::PrimaryStairs => self.primary_stairs,
            DecisionComponent::SupportFastBreakReturn => self.support_fast_break_return,
            DecisionComponent::SupportPullback => self.support_pullback,
            DecisionComponent::SupportVolume => self.support_volume,
            DecisionComponent::SupportExtreme => self.support_extreme,
            DecisionComponent::SupportTrend => self.support_trend,
        }
    }

    pub fn primary_enabled(self, kind: PrimaryKind) -> bool {
        match kind {
            PrimaryKind::Stairs => self.primary_stairs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PrimaryKind {
    Stairs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PrimaryPhase {
    Building,
    Pullback,
    Exhaustion,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrimaryState {
    pub kind: PrimaryKind,
    pub direction: i8,
    pub phase: PrimaryPhase,
    pub started_at_ms: u64,
    pub best_favorable_pct: f64,
    pub last_price: f64,
    pub volume_climax: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SupportEvidence {
    pub fast_break_return: bool,
    pub pullback: bool,
    pub volume_climax: bool,
    pub trend: bool,
    pub favorable_extreme_updated: bool,
    pub favorable_extreme_failed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DecisionState {
    pub primary: Option<PrimaryState>,
    pub support: SupportEvidence,
    pub trace: PrimaryRuleTrace,
}

/// Latest primary-rule evaluation. This is intentionally stateful rather than
/// emitted only when an entry opens: a wait is useful information too, since
/// it tells us whether stairs had no tradeable route or whether the
/// direction reservation blocked an opposite route.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PrimaryRuleTrace {
    pub now_ms: u64,
    pub stairs_up_candidates: u32,
    pub stairs_up_tradeable: u32,
    pub stairs_down_candidates: u32,
    pub stairs_down_tradeable: u32,
    pub stairs_tradeable: u32,
    pub direction_lock: Option<i8>,
    pub blocked_by_direction_lock: u32,
    pub primary_before: Option<PrimaryKind>,
    pub primary_after: Option<PrimaryKind>,
    pub selected_index: Option<usize>,
    pub selected_direction: i8,
    pub ended_by_extreme: bool,
    pub ended_by_protected_swing: bool,
    pub staircase_lifecycle: Option<StaircaseLifecycle>,
    pub staircase_transition: Option<StaircaseTransition>,
    pub direction_lock_reset_primary: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandidateView {
    pub index: usize,
    pub pattern_kind: MrcPatternKind,
    pub score: f64,
    pub entry: bool,
    pub direction: i8,
    pub volume_gain: f64,
    pub realized_favorable_pct: Option<f64>,
    pub realized_adverse_pct: Option<f64>,
    pub market_price: f64,
    pub staircase_lifecycle: Option<StaircaseLifecycle>,
    pub staircase_transition: Option<StaircaseTransition>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecisionResult {
    pub selected_index: Option<usize>,
    pub primary_kind: Option<PrimaryKind>,
    pub primary_direction: i8,
    pub primary_phase: Option<PrimaryPhase>,
    pub support: SupportEvidence,
}

impl DecisionState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn reconfigure(&mut self, components: DecisionComponents) {
        if self
            .primary
            .is_some_and(|primary| !components.primary_enabled(primary.kind))
        {
            self.primary = None;
        }
        self.support = SupportEvidence::default();
    }

    pub fn choose(
        &mut self,
        components: DecisionComponents,
        candidates: &[CandidateView],
        now_ms: u64,
    ) -> DecisionResult {
        self.choose_with_direction_lock(components, candidates, None, now_ms)
    }

    pub fn choose_with_direction_lock(
        &mut self,
        components: DecisionComponents,
        candidates: &[CandidateView],
        direction_lock: Option<i8>,
        now_ms: u64,
    ) -> DecisionResult {
        let primary_before = self.primary;
        let direction_lock_reset_primary = direction_lock.is_some_and(|direction| {
            self.primary
                .is_some_and(|primary| primary.direction != direction)
        });
        if direction_lock_reset_primary {
            self.primary = None;
        }
        let current = self.primary;
        let mut trace = primary_trace(
            components,
            candidates,
            direction_lock,
            primary_before,
            now_ms,
        );
        trace.direction_lock_reset_primary = direction_lock_reset_primary;
        let support = self.support(components, candidates, current);
        let ended_by_extreme = current.is_some_and(|current| {
            current.kind == PrimaryKind::Stairs && support.favorable_extreme_failed
        });
        let ended_by_protected_swing = current.is_some_and(|current| {
            current.kind == PrimaryKind::Stairs
                && candidates.iter().any(|candidate| {
                    primary_kind(candidate.pattern_kind) == Some(PrimaryKind::Stairs)
                        && candidate.direction == current.direction
                        && candidate.staircase_lifecycle == Some(StaircaseLifecycle::Invalidated)
                })
        });
        let ended = current.is_some_and(|current| {
            !components.primary_enabled(current.kind)
                || ended_by_extreme
                || ended_by_protected_swing
        });
        trace.ended_by_extreme = ended_by_extreme;
        trace.ended_by_protected_swing = ended_by_protected_swing;

        if ended {
            self.primary = None;
        }

        let candidate = if let Some(current) = self.primary {
            candidates
                .iter()
                .filter(|candidate| candidate_allowed(candidate, components, direction_lock))
                .filter_map(|candidate| {
                    (primary_kind(candidate.pattern_kind) == Some(current.kind)
                        && candidate.direction == current.direction)
                        .then_some(candidate)
                })
                .max_by(candidate_order)
                .or_else(|| {
                    // Keep the reserved direction while allowing a fresh
                    // staircase route to replace an exhausted primary state.
                    candidates
                        .iter()
                        .filter(|candidate| {
                            candidate_allowed(candidate, components, direction_lock)
                        })
                        .filter_map(|candidate| {
                            (primary_kind(candidate.pattern_kind)
                                .is_some_and(|kind| components.primary_enabled(kind))
                                && candidate.direction != 0)
                                .then_some(candidate)
                        })
                        .max_by(candidate_order)
                })
        } else {
            candidates
                .iter()
                .filter(|candidate| candidate_allowed(candidate, components, direction_lock))
                .filter_map(|candidate| {
                    let kind = primary_kind(candidate.pattern_kind)
                        .filter(|kind| components.primary_enabled(*kind))?;
                    if ended
                        && current.is_some_and(|previous| {
                            previous.kind == kind && previous.direction == candidate.direction
                        })
                    {
                        return None;
                    }
                    Some(candidate)
                })
                .max_by(candidate_order)
        };

        if let Some(candidate) = candidates.iter().find(|candidate| {
            primary_kind(candidate.pattern_kind) == Some(PrimaryKind::Stairs)
                && candidate.staircase_lifecycle.is_some()
        }) {
            trace.staircase_lifecycle = candidate.staircase_lifecycle;
            trace.staircase_transition = candidate.staircase_transition;
        }

        let Some(candidate) = candidate else {
            self.support = support;
            trace.primary_after = self.primary.map(|primary| primary.kind);
            trace.reason = if trace.blocked_by_direction_lock > 0 && trace.stairs_tradeable == 0 {
                "direction-lock-wait".to_string()
            } else if ended {
                "primary-ended-awaiting-rearm".to_string()
            } else if self.primary.is_some() {
                "primary-awaiting-same-direction-route".to_string()
            } else {
                "no-tradeable-primary-route".to_string()
            };
            self.trace = trace;
            return DecisionResult {
                selected_index: None,
                primary_kind: self.primary.map(|primary| primary.kind),
                primary_direction: self.primary.map(|primary| primary.direction).unwrap_or(0),
                primary_phase: self.primary.map(|primary| primary.phase),
                support,
            };
        };

        let kind = primary_kind(candidate.pattern_kind).expect("primary candidate kind");
        let direction = candidate.direction;
        let replace_primary = self
            .primary
            .is_none_or(|primary| primary.kind != kind || primary.direction != direction);
        if replace_primary {
            self.primary = Some(PrimaryState {
                kind,
                direction,
                phase: if support.pullback {
                    PrimaryPhase::Pullback
                } else if support.volume_climax {
                    PrimaryPhase::Exhaustion
                } else {
                    PrimaryPhase::Building
                },
                started_at_ms: now_ms,
                best_favorable_pct: 0.0,
                last_price: candidate.market_price,
                volume_climax: support.volume_climax,
            });
        } else if let Some(current) = self.primary.as_mut() {
            current.phase = if support.pullback {
                PrimaryPhase::Pullback
            } else if support.volume_climax || current.volume_climax {
                PrimaryPhase::Exhaustion
            } else {
                PrimaryPhase::Building
            };
            current.last_price = candidate.market_price;
            current.volume_climax |= support.volume_climax;
        }

        if let Some(current) = self.primary.as_mut() {
            if let Some(favorable) = candidate
                .realized_favorable_pct
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                current.best_favorable_pct = current.best_favorable_pct.max(favorable);
            }
        }
        self.support = support;
        trace.primary_after = Some(kind);
        trace.selected_index = Some(candidate.index);
        trace.selected_direction = direction;
        trace.staircase_lifecycle = candidate.staircase_lifecycle;
        trace.staircase_transition = candidate.staircase_transition;
        trace.reason = "selected-primary-route".to_string();
        self.trace = trace;
        DecisionResult {
            selected_index: Some(candidate.index),
            primary_kind: Some(kind),
            primary_direction: direction,
            primary_phase: self.primary.map(|primary| primary.phase),
            support,
        }
    }

    fn support(
        &self,
        components: DecisionComponents,
        candidates: &[CandidateView],
        primary: Option<PrimaryState>,
    ) -> SupportEvidence {
        let primary_direction = primary.map(|primary| primary.direction);
        let fbr = components.enabled(DecisionComponent::SupportFastBreakReturn)
            && candidates
                .iter()
                .any(|candidate| candidate.pattern_kind == MrcPatternKind::FastBreakReturn);
        let pullback = fbr
            && components.enabled(DecisionComponent::SupportPullback)
            && primary.is_some_and(|primary| {
                candidates.iter().any(|candidate| {
                    candidate.pattern_kind == MrcPatternKind::FastBreakReturn
                        && candidate.direction == -primary.direction
                })
            });
        let volume_climax = components.enabled(DecisionComponent::SupportVolume)
            && candidates.iter().any(|candidate| {
                candidate.volume_gain.is_finite()
                    && candidate.volume_gain > crate::observer::VOLUME_GAIN_THRESHOLD
            });
        let current_candidate = primary.and_then(|primary| {
            candidates
                .iter()
                .filter(|candidate| candidate.entry)
                .filter(|candidate| {
                    primary_kind(candidate.pattern_kind) == Some(primary.kind)
                        && candidate.direction == primary.direction
                })
                .max_by(candidate_order)
        });
        let favorable_extreme_updated = components.enabled(DecisionComponent::SupportExtreme)
            && primary.is_some_and(|primary| {
                current_candidate
                    .and_then(|candidate| candidate.realized_favorable_pct)
                    .is_some_and(|value| value.is_finite() && value > primary.best_favorable_pct)
            });
        let favorable_extreme_failed = components.enabled(DecisionComponent::SupportExtreme)
            && primary.is_some_and(|primary| {
                primary.best_favorable_pct > 0.0
                    && current_candidate.is_some_and(|candidate| {
                        let favorable = candidate.realized_favorable_pct.unwrap_or(0.0);
                        let adverse = candidate.realized_adverse_pct.unwrap_or(0.0).abs();
                        favorable.is_finite()
                            && adverse.is_finite()
                            && favorable <= primary.best_favorable_pct
                            && adverse >= favorable
                    })
            });
        let trend = components.enabled(DecisionComponent::SupportTrend)
            && candidates.iter().any(|candidate| {
                candidate.pattern_kind == MrcPatternKind::TrendingCycle
                    && primary_direction.is_none_or(|direction| candidate.direction == direction)
            });
        SupportEvidence {
            fast_break_return: fbr,
            pullback,
            volume_climax,
            trend,
            favorable_extreme_updated,
            favorable_extreme_failed,
        }
    }
}

fn candidate_allowed(
    candidate: &CandidateView,
    components: DecisionComponents,
    direction_lock: Option<i8>,
) -> bool {
    candidate.entry
        && !matches!(
            candidate.staircase_lifecycle,
            Some(StaircaseLifecycle::Invalidated)
        )
        && primary_kind(candidate.pattern_kind).is_some_and(|kind| components.primary_enabled(kind))
        && direction_lock.is_none_or(|direction| candidate.direction == direction)
}

fn primary_trace(
    components: DecisionComponents,
    candidates: &[CandidateView],
    direction_lock: Option<i8>,
    primary_before: Option<PrimaryState>,
    now_ms: u64,
) -> PrimaryRuleTrace {
    let mut trace = PrimaryRuleTrace {
        now_ms,
        direction_lock,
        primary_before: primary_before.map(|primary| primary.kind),
        ..PrimaryRuleTrace::default()
    };
    for candidate in candidates {
        let Some(kind) = primary_kind(candidate.pattern_kind) else {
            continue;
        };
        let direction_allowed =
            direction_lock.is_none_or(|direction| candidate.direction == direction);
        match (kind, candidate.direction) {
            (PrimaryKind::Stairs, direction) if direction > 0 => trace.stairs_up_candidates += 1,
            (PrimaryKind::Stairs, direction) if direction < 0 => trace.stairs_down_candidates += 1,
            _ => {}
        }
        if candidate.entry && !direction_allowed {
            trace.blocked_by_direction_lock += 1;
        }
        if candidate_allowed(candidate, components, direction_lock) {
            trace.stairs_tradeable += 1;
            if candidate.direction > 0 {
                trace.stairs_up_tradeable += 1;
            } else if candidate.direction < 0 {
                trace.stairs_down_tradeable += 1;
            }
        }
    }
    trace
}

fn primary_kind(kind: MrcPatternKind) -> Option<PrimaryKind> {
    match kind {
        MrcPatternKind::StairsUp | MrcPatternKind::StairsDown => Some(PrimaryKind::Stairs),
        _ => None,
    }
}

fn candidate_order(left: &&CandidateView, right: &&CandidateView) -> std::cmp::Ordering {
    left.score
        .partial_cmp(&right.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left.index.cmp(&right.index))
}
