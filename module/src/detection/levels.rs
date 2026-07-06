use crate::model::IndicatorSnapshot;

const LEVEL_TOLERANCE_PCT: f64 = 0.0015;

#[derive(Debug, Clone, Default)]
pub struct LevelBook {
    pub above: Vec<f64>,
    pub below: Vec<f64>,
}

impl LevelBook {
    pub fn nearest_above(&self, price: f64) -> Option<f64> {
        self.above
            .iter()
            .copied()
            .filter(|level| *level > price)
            .min_by(|a, b| a.total_cmp(b))
    }

    pub fn nearest_below(&self, price: f64) -> Option<f64> {
        self.below
            .iter()
            .copied()
            .filter(|level| *level < price)
            .max_by(|a, b| a.total_cmp(b))
    }
}

fn add_level(levels: &mut Vec<f64>, value: f64) {
    if !value.is_finite() || value <= 0.0 {
        return;
    }
    if levels.iter().any(|existing| (existing - value).abs() < f64::EPSILON) {
        return;
    }
    levels.push(value);
}

fn cluster(levels: &[f64], tolerance: f64) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for level in levels.iter().copied() {
        if let Some(last) = out.last_mut() {
            if (level - *last).abs() <= tolerance {
                *last = (*last + level) / 2.0;
                continue;
            }
        }
        out.push(level);
    }
    out
}

pub fn structural_levels(snapshot: &IndicatorSnapshot) -> LevelBook {
    let price = snapshot.tf1m.candle.close.max(0.0);
    let mut above = Vec::new();
    let mut below = Vec::new();

    for tf in [
        snapshot.tf1m,
        snapshot.tf5m,
        snapshot.tf15m,
        snapshot.tf1h,
    ] {
        add_level(&mut above, tf.mrc.up_small);
        add_level(&mut above, tf.mrc.up_inner);
        add_level(&mut above, tf.mrc.up_outer);
        add_level(&mut below, tf.mrc.down_small);
        add_level(&mut below, tf.mrc.down_inner);
        add_level(&mut below, tf.mrc.down_outer);
        add_level(&mut above, tf.dp_zee);
        add_level(&mut above, tf.dp_zee1);
        add_level(&mut above, tf.zz_sell_price);
        add_level(&mut above, tf.zz_break_price);
        add_level(&mut below, tf.dp_zee);
        add_level(&mut below, tf.dp_zee1);
        add_level(&mut below, tf.zz_buy_price);
        add_level(&mut below, tf.zz_break_price);
        add_level(&mut above, tf.candle.high);
        add_level(&mut below, tf.candle.low);
    }

    let tolerance = (price * LEVEL_TOLERANCE_PCT).max(0.01);
    above.sort_by(|a, b| a.total_cmp(b));
    below.sort_by(|a, b| b.total_cmp(a));

    LevelBook {
        above: cluster(&above, tolerance),
        below: cluster(&below, tolerance),
    }
}
