use crate::model::{MarketState, TimeframeSnapshot};
use crate::state::DirectionLockState;

fn confirmed_direction(snapshot: &TimeframeSnapshot) -> MarketState {
    MarketState::from_dir_pair(snapshot.dp_dir, snapshot.zz_dir)
}

fn fallback_break_price(snapshot: &TimeframeSnapshot) -> f64 {
    if snapshot.zz_break_price > 0.0 {
        snapshot.zz_break_price
    } else if snapshot.dp_zee1 > 0.0 {
        snapshot.dp_zee1
    } else if snapshot.dp_zee > 0.0 {
        snapshot.dp_zee
    } else {
        snapshot.candle.close
    }
}

fn is_broken(lock: &DirectionLockState, snapshot: &TimeframeSnapshot, price: f64) -> bool {
    match lock.direction {
        MarketState::Long => price < lock.break_price || snapshot.candle.low < lock.break_price,
        MarketState::Short => price > lock.break_price || snapshot.candle.high > lock.break_price,
        MarketState::Sideways => false,
    }
}

pub fn refresh(lock: &mut DirectionLockState, snapshot: &TimeframeSnapshot, price: f64, tick: u64) {
    let confirmed = confirmed_direction(snapshot);
    if confirmed == MarketState::Sideways {
        lock.last_tick = tick;
        if lock.direction != MarketState::Sideways {
            lock.reason = "dpzz-freeze".to_string();
        }
        return;
    }

    let break_price = fallback_break_price(snapshot);
    if lock.direction == MarketState::Sideways {
        lock.direction = confirmed;
        lock.break_price = break_price;
        lock.last_tick = tick;
        lock.reason = "dpzz-lock".to_string();
        return;
    }

    if lock.direction == confirmed {
        lock.break_price = break_price;
        lock.last_tick = tick;
        lock.reason = "dpzz-hold".to_string();
        return;
    }

    if is_broken(lock, snapshot, price) {
        lock.direction = confirmed;
        lock.break_price = break_price;
        lock.last_tick = tick;
        lock.reason = "zigzag-break".to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, TimeframeSnapshot};

    fn tf(dp: i8, zz: i8, break_price: f64, close: f64, low: f64, high: f64) -> TimeframeSnapshot {
        TimeframeSnapshot {
            candle: Candle {
                open: close,
                high,
                low,
                close,
            },
            mrc: MrcBands {
                down_big: 0.0,
                down_outer: 0.0,
                down_small: 0.0,
                down_inner: 0.0,
                mean_line: 0.0,
                up_inner: 0.0,
                up_small: 0.0,
                up_outer: 0.0,
                up_big: 0.0,
                current_cross: 0.0,
                mean_line_slope: 0.0,
            },
            dp_dir: dp,
            zz_dir: zz,
            zz_break_price: break_price,
            dp_zee1: break_price,
            dp_zee: break_price,
            ..Default::default()
        }
    }

    #[test]
    fn locks_on_agreement() {
        let mut lock = DirectionLockState::default();
        let snapshot = tf(1, 1, 100.0, 101.0, 100.5, 101.5);
        refresh(&mut lock, &snapshot, 101.0, 1);
        assert_eq!(lock.direction, MarketState::Long);
        assert_eq!(lock.break_price, 100.0);
        assert_eq!(lock.reason, "dpzz-lock");
    }

    #[test]
    fn keeps_direction_until_break() {
        let mut lock = DirectionLockState {
            direction: MarketState::Long,
            break_price: 100.0,
            last_tick: 1,
            reason: "dpzz-lock".to_string(),
        };
        let snapshot = tf(-1, -1, 99.0, 99.5, 99.0, 100.5);
        refresh(&mut lock, &snapshot, 99.5, 2);
        assert_eq!(lock.direction, MarketState::Short);
        assert_eq!(lock.break_price, 99.0);
        assert_eq!(lock.reason, "zigzag-break");
    }
}
