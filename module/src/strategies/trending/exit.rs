use crate::model::{MrcBands, PositionState};
use crate::orders::tp_mark;
use crate::state::EngineState;
use lte_strategy_bridge::abi::{Direction, ModuleUpdatePosition};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Update {
    pub stop_loss: Option<f64>,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    NoPosition,
    Hold,
    Update(Update),
}

pub fn evaluate(
    position: PositionState,
    entry_price: f64,
    market_price: f64,
    stop_loss: f64,
    mrc: MrcBands,
) -> Decision {
    if position == PositionState::Flat {
        return Decision::NoPosition;
    }
    let lines = mrc.lines();
    if !mrc.ordered()
        || !lines.iter().all(|line| line.is_finite() && *line > 0.0)
        || !entry_price.is_finite()
        || entry_price <= 0.0
        || !market_price.is_finite()
        || market_price <= 0.0
    {
        return Decision::Hold;
    }

    let direction = if position == PositionState::Long {
        1
    } else {
        -1
    };
    let initial_stop = if valid_stop_price(stop_loss) {
        None
    } else {
        outer_stop_line(lines, direction)
            .filter(|(_, price)| is_adverse_stop(*price, entry_price, direction))
    };
    let current_stop = if valid_stop_price(stop_loss) {
        Some(stop_loss)
    } else {
        initial_stop.map(|(_, price)| price)
    };
    let trailing_stop = trailing_line(lines, market_price, direction).filter(|(_, price)| {
        improves_entry_stop(*price, entry_price, direction)
            && current_stop.is_some_and(|current| improves_stop(*price, current, direction))
    });
    let stop = trailing_stop.or(initial_stop);
    if stop.is_none() {
        return Decision::Hold;
    }
    Decision::Update(Update {
        stop_loss: stop.map(|(_, price)| price),
        reason: if trailing_stop.is_some() {
            "mrc-trailing-stop"
        } else {
            "mrc-exit-levels"
        },
    })
}

pub fn protect_handoff_stop_loss(
    position: PositionState,
    entry_price: f64,
    current_stop: f64,
    candidate_stop: f64,
    mrc: MrcBands,
) -> Option<(f64, bool)> {
    if position == PositionState::Flat || !entry_price.is_finite() || entry_price <= 0.0 {
        return None;
    }
    let direction = if position == PositionState::Long {
        1
    } else {
        -1
    };
    let structural_stop = if valid_stop_price(current_stop) {
        Some(current_stop)
    } else {
        let lines = mrc.lines();
        (mrc.ordered() && lines.iter().all(|line| line.is_finite() && *line > 0.0))
            .then(|| outer_stop_line(lines, direction))
            .flatten()
            .filter(|(_, price)| is_adverse_stop(*price, entry_price, direction))
            .map(|(_, price)| price)
    };
    let current =
        structural_stop.or_else(|| valid_stop_price(candidate_stop).then_some(candidate_stop))?;
    let candidate_is_safe = valid_stop_price(candidate_stop)
        && improves_entry_stop(candidate_stop, entry_price, direction)
        && improves_stop(candidate_stop, current, direction);
    Some((
        if candidate_is_safe {
            candidate_stop
        } else {
            current
        },
        candidate_is_safe,
    ))
}

pub fn execute(state: &mut EngineState, update: Update) -> ModuleUpdatePosition {
    if let Some(price) = update.stop_loss {
        state.position.stop_loss = price;
        state.position.stop_loss_reason = update.reason.to_string();
    }
    ModuleUpdatePosition {
        direction: if state.position.side == PositionState::Long {
            Direction::Long
        } else {
            Direction::Short
        },
        enter_price: None,
        take_profit: None,
        stop_loss: update.stop_loss,
    }
}

fn valid_stop_price(price: f64) -> bool {
    price.is_finite() && price > 0.0
}

fn is_adverse_stop(price: f64, entry_price: f64, direction: i8) -> bool {
    valid_stop_price(price)
        && if direction > 0 {
            price < entry_price
        } else {
            price > entry_price
        }
}

fn improves_stop(candidate: f64, current: f64, direction: i8) -> bool {
    if !current.is_finite() || current <= 0.0 {
        return true;
    }
    if direction > 0 {
        candidate > current
    } else {
        candidate < current
    }
}

fn improves_entry_stop(candidate: f64, entry_price: f64, direction: i8) -> bool {
    if direction > 0 {
        candidate > entry_price
    } else {
        candidate < entry_price
    }
}

fn outer_stop_line(lines: [f64; 9], direction: i8) -> Option<(u8, f64)> {
    let index = if direction > 0 { 0 } else { lines.len() - 1 };
    lines.get(index).copied().map(|price| (index as u8, price))
}

fn nearest_line(lines: [f64; 9], price: f64, direction: i8) -> Option<(u8, f64)> {
    lines
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, level)| {
            if direction > 0 {
                *level > price
            } else {
                *level < price
            }
        })
        .min_by(|left, right| (left.1 - price).abs().total_cmp(&(right.1 - price).abs()))
        .map(|(line, price)| (line as u8, price))
}

fn trailing_line(lines: [f64; 9], price: f64, direction: i8) -> Option<(u8, f64)> {
    let (nearest_index, _) = nearest_line(lines, price, -direction)?;
    let stop_index = if direction > 0 {
        nearest_index.checked_sub(1)?
    } else {
        nearest_index.checked_add(1)?
    };
    lines
        .get(stop_index as usize)
        .copied()
        .map(|price| (stop_index, price))
}

pub(crate) fn all_tp_marks(direction: Direction) -> Vec<String> {
    let base = tp_mark(direction);
    std::iter::once(base.clone())
        .chain(std::iter::once(format!("{base}-far")))
        .chain((1..=3).map(move |index| format!("{base}-{index}")))
        .collect()
}
