use crate::model::IndicatorSnapshot;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcPriceZoneSide {
    Lower,
    Upper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MrcLineDistance {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrcPriceWindow {
    pub side: MrcPriceZoneSide,
    pub distance: MrcLineDistance,
    pub lower: f64,
    pub upper: f64,
    pub width: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelSource {
    Tf1m,
    Tf5m,
    Tf15m,
    Tf1h,
}

impl LevelSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tf1m => "1m",
            Self::Tf5m => "5m",
            Self::Tf15m => "15m",
            Self::Tf1h => "1h",
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            Self::Tf1m => 0,
            Self::Tf5m => 1,
            Self::Tf15m => 2,
            Self::Tf1h => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelKind {
    MrcUpSmall,
    MrcUpInner,
    MrcUpOuter,
    MrcDownSmall,
    MrcDownInner,
    MrcDownOuter,
    DpZee,
    DpZee1,
    ZzSellPrice,
    ZzBreakPrice,
    CandleHigh,
    CandleLow,
}

impl LevelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MrcUpSmall => "mrc_up_small",
            Self::MrcUpInner => "mrc_up_inner",
            Self::MrcUpOuter => "mrc_up_outer",
            Self::MrcDownSmall => "mrc_down_small",
            Self::MrcDownInner => "mrc_down_inner",
            Self::MrcDownOuter => "mrc_down_outer",
            Self::DpZee => "dp_zee",
            Self::DpZee1 => "dp_zee1",
            Self::ZzSellPrice => "zz_sell",
            Self::ZzBreakPrice => "zz_break",
            Self::CandleHigh => "candle_high",
            Self::CandleLow => "candle_low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelSide {
    Above,
    Below,
}

impl LevelSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Below => "below",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelPoint {
    pub source: LevelSource,
    pub kind: LevelKind,
    pub side: LevelSide,
    pub value: f64,
}

impl LevelPoint {
    pub fn label(&self) -> String {
        format!(
            "{}:{}:{}",
            self.source.as_str(),
            self.side.as_str(),
            self.kind.as_str()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelGapSummary {
    pub nearest_above: Option<LevelPoint>,
    pub nearest_below: Option<LevelPoint>,
    pub nearest_above_gap: Option<f64>,
    pub nearest_below_gap: Option<f64>,
    pub next_above_gap: Option<f64>,
    pub next_below_gap: Option<f64>,
    pub span: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct LevelBook {
    pub above: Vec<LevelPoint>,
    pub below: Vec<LevelPoint>,
}

impl LevelBook {
    fn compare_points(a: &LevelPoint, b: &LevelPoint) -> Ordering {
        a.value
            .total_cmp(&b.value)
            .then(a.source.priority().cmp(&b.source.priority()))
            .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
    }

    pub fn nearest_above(&self, price: f64) -> Option<f64> {
        self.nearest_above_point(price).map(|point| point.value)
    }

    pub fn nearest_below(&self, price: f64) -> Option<f64> {
        self.nearest_below_point(price).map(|point| point.value)
    }

    pub fn nearest_above_point(&self, price: f64) -> Option<LevelPoint> {
        self.above
            .iter()
            .copied()
            .filter(|point| point.value > price)
            .min_by(Self::compare_points)
    }

    pub fn nearest_below_point(&self, price: f64) -> Option<LevelPoint> {
        self.below
            .iter()
            .copied()
            .filter(|point| point.value < price)
            .max_by(Self::compare_points)
    }

    pub fn nearby_above(&self, price: f64, limit: usize) -> Vec<LevelPoint> {
        let mut points: Vec<LevelPoint> = self
            .above
            .iter()
            .copied()
            .filter(|point| point.value > price)
            .collect();
        points.sort_by(Self::compare_points);
        points.truncate(limit);
        points
    }

    pub fn nearby_below(&self, price: f64, limit: usize) -> Vec<LevelPoint> {
        let mut points: Vec<LevelPoint> = self
            .below
            .iter()
            .copied()
            .filter(|point| point.value < price)
            .collect();
        points.sort_by(|a, b| Self::compare_points(b, a));
        points.truncate(limit);
        points
    }

    pub fn gap_summary(&self, price: f64) -> LevelGapSummary {
        let nearest_above = self.nearest_above_point(price);
        let nearest_below = self.nearest_below_point(price);

        let next_above_gap = nearest_above.and_then(|first| {
            self.above
                .iter()
                .copied()
                .filter(|point| point.value > first.value)
                .min_by(Self::compare_points)
                .map(|next| next.value - first.value)
        });
        let next_below_gap = nearest_below.and_then(|first| {
            self.below
                .iter()
                .copied()
                .filter(|point| point.value < first.value)
                .max_by(Self::compare_points)
                .map(|next| first.value - next.value)
        });

        LevelGapSummary {
            nearest_above_gap: nearest_above.map(|point| point.value - price),
            nearest_below_gap: nearest_below.map(|point| price - point.value),
            span: match (nearest_above, nearest_below) {
                (Some(above), Some(below)) if above.value > below.value => {
                    Some(above.value - below.value)
                }
                _ => None,
            },
            nearest_above,
            nearest_below,
            next_above_gap,
            next_below_gap,
        }
    }
}

fn push_level(levels: &mut Vec<LevelPoint>, point: LevelPoint) {
    if !point.value.is_finite() || point.value <= 0.0 {
        return;
    }
    levels.push(point);
}

fn sort_levels(levels: &mut [LevelPoint]) {
    levels.sort_by(LevelBook::compare_points);
}

pub fn structural_levels(snapshot: &IndicatorSnapshot) -> LevelBook {
    let mut above = Vec::new();
    let mut below = Vec::new();

    for (source, tf) in [
        (LevelSource::Tf1m, snapshot.tf1m),
        (LevelSource::Tf5m, snapshot.tf5m),
        (LevelSource::Tf15m, snapshot.tf15m),
        (LevelSource::Tf1h, snapshot.tf1h),
    ] {
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::MrcUpSmall,
                side: LevelSide::Above,
                value: tf.mrc.up_small,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::MrcUpInner,
                side: LevelSide::Above,
                value: tf.mrc.up_inner,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::MrcUpOuter,
                side: LevelSide::Above,
                value: tf.mrc.up_outer,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::MrcDownSmall,
                side: LevelSide::Below,
                value: tf.mrc.down_small,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::MrcDownInner,
                side: LevelSide::Below,
                value: tf.mrc.down_inner,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::MrcDownOuter,
                side: LevelSide::Below,
                value: tf.mrc.down_outer,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::DpZee,
                side: LevelSide::Above,
                value: tf.dp_zee,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::DpZee1,
                side: LevelSide::Above,
                value: tf.dp_zee1,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::ZzSellPrice,
                side: LevelSide::Above,
                value: tf.zz_sell_price,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::ZzBreakPrice,
                side: LevelSide::Above,
                value: tf.zz_break_price,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::DpZee,
                side: LevelSide::Below,
                value: tf.dp_zee,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::DpZee1,
                side: LevelSide::Below,
                value: tf.dp_zee1,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::ZzSellPrice,
                side: LevelSide::Below,
                value: tf.zz_buy_price,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::ZzBreakPrice,
                side: LevelSide::Below,
                value: tf.zz_break_price,
            },
        );
        push_level(
            &mut above,
            LevelPoint {
                source,
                kind: LevelKind::CandleHigh,
                side: LevelSide::Above,
                value: tf.candle.high,
            },
        );
        push_level(
            &mut below,
            LevelPoint {
                source,
                kind: LevelKind::CandleLow,
                side: LevelSide::Below,
                value: tf.candle.low,
            },
        );
    }

    sort_levels(&mut above);
    sort_levels(&mut below);

    LevelBook { above, below }
}

pub fn mrc_price_window(mrc: &crate::model::MrcBands, price: f64) -> Option<MrcPriceWindow> {
    if !mrc.ordered() || !price.is_finite() || price <= 0.0 {
        return None;
    }

    let (side, distance, lower, upper) = if price >= mrc.mean_line {
        if price <= mrc.up_inner {
            (
                MrcPriceZoneSide::Upper,
                MrcLineDistance::One,
                mrc.mean_line,
                mrc.up_inner,
            )
        } else if price <= mrc.up_small {
            (
                MrcPriceZoneSide::Upper,
                MrcLineDistance::One,
                mrc.up_inner,
                mrc.up_small,
            )
        } else if price <= mrc.up_outer {
            (
                MrcPriceZoneSide::Upper,
                MrcLineDistance::One,
                mrc.up_small,
                mrc.up_outer,
            )
        } else {
            (
                MrcPriceZoneSide::Upper,
                MrcLineDistance::Two,
                mrc.up_small,
                mrc.up_big,
            )
        }
    } else if price >= mrc.down_inner {
        (
            MrcPriceZoneSide::Lower,
            MrcLineDistance::One,
            mrc.down_inner,
            mrc.mean_line,
        )
    } else if price >= mrc.down_small {
        (
            MrcPriceZoneSide::Lower,
            MrcLineDistance::One,
            mrc.down_small,
            mrc.down_inner,
        )
    } else if price >= mrc.down_outer {
        (
            MrcPriceZoneSide::Lower,
            MrcLineDistance::One,
            mrc.down_outer,
            mrc.down_small,
        )
    } else {
        (
            MrcPriceZoneSide::Lower,
            MrcLineDistance::Two,
            mrc.down_big,
            mrc.down_small,
        )
    };

    Some(MrcPriceWindow {
        side,
        distance,
        lower,
        upper,
        width: (upper - lower).abs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, MrcBands, TimeframeSnapshot};

    fn tf(close: f64, offset: f64) -> TimeframeSnapshot {
        TimeframeSnapshot {
            candle: Candle {
                open: close,
                high: close + offset,
                low: close - offset,
                close,
            },
            mrc: MrcBands {
                down_big: close - offset * 4.0,
                down_outer: close - offset * 3.0,
                down_small: close - offset * 2.0,
                down_inner: close - offset,
                mean_line: close,
                up_inner: close + offset,
                up_small: close + offset * 2.0,
                up_outer: close + offset * 3.0,
                up_big: close + offset * 4.0,
                current_cross: 0.0,
                mean_line_slope: 0.0,
            },
            dp_zee: close - offset * 0.5,
            dp_zee1: close + offset * 0.5,
            zz_buy_price: close - offset * 0.75,
            zz_sell_price: close + offset * 0.75,
            zz_break_price: close + offset * 1.25,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_all_levels_without_flat_clustering() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0, 1.0),
            tf5m: tf(100.0, 2.0),
            tf15m: tf(100.0, 3.0),
            tf1h: tf(100.0, 4.0),
            ..Default::default()
        };
        let book = structural_levels(&snapshot);
        assert!(book.above.len() > 4);
        assert!(book.below.len() > 4);
        assert!(book.nearest_above(100.0).is_some());
        assert!(book.nearest_below(100.0).is_some());
    }

    #[test]
    fn gap_summary_reports_near_levels() {
        let snapshot = IndicatorSnapshot {
            tf1m: tf(100.0, 1.0),
            tf5m: tf(100.0, 2.0),
            tf15m: tf(100.0, 3.0),
            tf1h: tf(100.0, 4.0),
            ..Default::default()
        };
        let book = structural_levels(&snapshot);
        let summary = book.gap_summary(100.0);
        assert!(summary.nearest_above_gap.unwrap_or(0.0) > 0.0);
        assert!(summary.nearest_below_gap.unwrap_or(0.0) > 0.0);
    }
}
