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
        bullish_structure: ordered && slope_sign > 0 && price >= mrc.mean_line && pzone >= 0.52,
        bearish_structure: ordered && slope_sign < 0 && price <= mrc.mean_line && pzone <= 0.48,
    }
}
