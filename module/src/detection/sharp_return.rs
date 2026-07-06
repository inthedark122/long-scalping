use crate::model::{Candle, MrcBands, RiskSnapshot};
use crate::state::SharpReturnState;

const RETURN_WINDOW_BARS: u64 = 3;

fn zone(mrc: &MrcBands, price: f64) -> i32 {
    if !mrc.ordered() {
        return 0;
    }
    mrc.lines().iter().filter(|line| price >= **line).count() as i32
}

pub fn update(
    memory: &mut SharpReturnState,
    candle: Candle,
    mrc: &MrcBands,
    bar_1m: u64,
) -> RiskSnapshot {
    if mrc.ordered() && candle.open > 0.0 {
        let down_cross = zone(mrc, candle.open) - zone(mrc, candle.low);
        let up_cross = zone(mrc, candle.high) - zone(mrc, candle.open);

        if !memory.active && down_cross >= 2 {
            memory.active = true;
            memory.direction = -1;
            memory.ref_price = candle.open;
            memory.armed_bar = bar_1m;
        } else if !memory.active && up_cross >= 2 {
            memory.active = true;
            memory.direction = 1;
            memory.ref_price = candle.open;
            memory.armed_bar = bar_1m;
        }
    }

    if memory.active {
        let returned = match memory.direction {
            -1 => candle.close >= memory.ref_price,
            1 => candle.close <= memory.ref_price,
            _ => false,
        };

        if returned {
            memory.active = false;
            memory.fail_streak = 0;
            memory.last_fail_price = 0.0;
        } else if bar_1m.saturating_sub(memory.armed_bar) >= RETURN_WINDOW_BARS {
            memory.active = false;
            memory.fail_streak = memory.fail_streak.saturating_add(1);
            memory.last_fail_price = memory.ref_price;
        }
    }

    if memory.last_fail_price > 0.0 {
        let recovered =
            candle.close >= memory.last_fail_price || candle.close <= memory.last_fail_price;
        if recovered && memory.fail_streak == 0 {
            memory.last_fail_price = 0.0;
        }
    }

    RiskSnapshot {
        fail_streak: memory.fail_streak,
        active: memory.active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mrc() -> MrcBands {
        MrcBands {
            down_big: 80.0,
            down_outer: 85.0,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line: 100.0,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope: 0.0,
        }
    }

    #[test]
    fn sharp_move_risk_flips_from_candle_and_mrc_only() {
        let mut state = SharpReturnState::default();

        let low_risk = update(
            &mut state,
            Candle {
                open: 100.0,
                high: 101.0,
                low: 88.0,
                close: 89.0,
            },
            &mrc(),
            1,
        );
        assert!(low_risk.active);
        assert_eq!(low_risk.fail_streak, 0);

        update(
            &mut state,
            Candle {
                open: 89.0,
                high: 90.0,
                low: 87.0,
                close: 88.0,
            },
            &mrc(),
            2,
        );
        let mid = update(
            &mut state,
            Candle {
                open: 88.0,
                high: 89.0,
                low: 86.0,
                close: 87.0,
            },
            &mrc(),
            4,
        );
        assert_eq!(mid.fail_streak, 1);

        update(
            &mut state,
            Candle {
                open: 100.0,
                high: 112.0,
                low: 99.0,
                close: 111.0,
            },
            &mrc(),
            5,
        );
        let high_risk = update(
            &mut state,
            Candle {
                open: 111.0,
                high: 113.0,
                low: 110.0,
                close: 112.0,
            },
            &mrc(),
            8,
        );
        assert_eq!(high_risk.fail_streak, 2);
    }
}
