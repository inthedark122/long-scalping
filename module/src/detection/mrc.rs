use crate::model::MrcBands;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct MrcSignal {
    pub ordered: bool,
    pub zone: i8,
    pub pzone: f64,
    pub slope_sign: i8,
    pub bullish_structure: bool,
    pub bearish_structure: bool,
}

pub fn analyze(mrc: &MrcBands, price: f64) -> MrcSignal {
    let ordered = mrc.ordered();
    let range = mrc.range();
    let pzone = if ordered && range > 0.0 {
        ((price - mrc.down_big) / range).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let slope_sign = if mrc.mean_line_slope > 0.0 {
        1
    } else if mrc.mean_line_slope < 0.0 {
        -1
    } else {
        0
    };
    let zone = if !ordered {
        0
    } else {
        mrc.lines().iter().filter(|line| price >= **line).count() as i8 - 4
    };

    MrcSignal {
        ordered,
        zone,
        pzone,
        slope_sign,
        bullish_structure: ordered
            && slope_sign > 0
            && ((price >= mrc.mean_line && pzone >= 0.52)
                || (price >= mrc.down_outer && pzone >= 0.35)),
        bearish_structure: ordered && slope_sign < 0 && price <= mrc.mean_line && pzone <= 0.48,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mrc(mean_line_slope: f64, _price: f64, down_outer: f64, mean_line: f64) -> MrcBands {
        MrcBands {
            down_big: 80.0,
            down_outer,
            down_small: 90.0,
            down_inner: 95.0,
            mean_line,
            up_inner: 105.0,
            up_small: 110.0,
            up_outer: 115.0,
            up_big: 120.0,
            current_cross: 0.0,
            mean_line_slope,
        }
    }

    #[test]
    fn bullish_structure_accepts_hold_above_down_outer() {
        let signal = analyze(&mrc(1.0, 95.0, 85.0, 100.0), 95.0);
        assert!(signal.bullish_structure);
    }

    #[test]
    fn bullish_structure_still_requires_positive_slope_and_ordered_bands() {
        let mut bands = mrc(1.0, 88.0, 85.0, 100.0);
        bands.down_outer = 101.0;
        let signal = analyze(&bands, 88.0);
        assert!(!signal.bullish_structure);
    }
}
