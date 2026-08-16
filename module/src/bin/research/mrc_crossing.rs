use long_scalping::detection::{mrc_crossing, vlines};
use long_scalping::state::{MrcCrossingModel, MrcCrossingState, VLineState, MRC_PATTERN_LENGTH};
use serde::Serialize;

use super::data::CandleRow;
use super::virtual_trade::{TradeSetup, VirtualTradeSimulator};
pub(crate) type TradeResult = super::virtual_trade::VirtualTradeResult;
pub(crate) type TradeSummary = super::virtual_trade::VirtualTradeSummary;
pub(crate) type VirtualPosition = super::virtual_trade::VirtualPosition;

const LINE_COUNT: usize = 9;
const LABELS: [&str; LINE_COUNT] = mrc_crossing::LINE_LABELS;

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CrossingEvent {
    pub(crate) time_ms: i64,
    pub(crate) line: usize,
    pub(crate) line_name: &'static str,
    pub(crate) direction: i8,
    pub(crate) price: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PatternReport {
    pub(crate) pattern: [usize; MRC_PATTERN_LENGTH],
    pub(crate) pattern_names: [&'static str; MRC_PATTERN_LENGTH],
    pub(crate) to: usize,
    pub(crate) to_name: &'static str,
    pub(crate) count: usize,
    pub(crate) probability: f64,
    pub(crate) average_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Forecast {
    pub(crate) source_line: Option<usize>,
    pub(crate) source_name: Option<&'static str>,
    pub(crate) condition: &'static str,
    pub(crate) sample_count: usize,
    pub(crate) probability_next_crossing: Option<f64>,
    pub(crate) estimated_time_ms: Option<f64>,
    pub(crate) likely_destinations: Vec<PatternReport>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnalysisReport {
    pub(crate) crossing_count: usize,
    pub(crate) pattern_count: usize,
    pub(crate) patterns: Vec<PatternReport>,
    pub(crate) latest_event: Option<CrossingEvent>,
    pub(crate) forecast: Forecast,
    pub(crate) events: Vec<CrossingEvent>,
    pub(crate) trade_summary: TradeSummary,
    pub(crate) trades: Vec<TradeResult>,
    pub(crate) open_trade: Option<VirtualPosition>,
}

#[derive(Debug, Clone)]
pub(crate) struct Analyzer {
    vlines: [VLineState; LINE_COUNT],
    crossing: MrcCrossingState,
    model: MrcCrossingModel,
    events: Vec<CrossingEvent>,
    trades: VirtualTradeSimulator,
    required_samples: u32,
}

impl Analyzer {
    pub(crate) fn new(required_samples: u32) -> Self {
        Self {
            vlines: [VLineState::default(); LINE_COUNT],
            crossing: MrcCrossingState::default(),
            model: MrcCrossingModel::default(),
            events: Vec::new(),
            trades: VirtualTradeSimulator::default(),
            required_samples,
        }
    }
}

impl Analyzer {
    pub(crate) fn observe(&mut self, row: &CandleRow, _hourly_ntps: &[i64]) {
        if !row.mrc.ordered() || !row.close.is_finite() || row.close <= 0.0 {
            return;
        }
        self.trades.update(row);
        let lines = row.mrc.lines();
        let crossed = vlines::update_with_events(
            &mut self.vlines,
            &mut self.crossing,
            lines,
            row.close,
            self.events.len() as u64 + 1,
            row.time_ms.max(0) as u64,
        );
        let observation =
            mrc_crossing::observe(&mut self.model, &crossed, lines, self.required_samples);
        for event in observation.events {
            let target_line = event.target_line;
            let event = CrossingEvent {
                time_ms: event.now_ms as i64,
                line: event.line_idx as usize,
                line_name: LABELS[event.line_idx as usize],
                direction: event.direction,
                price: event.line,
            };
            if let Some(target_line) = target_line {
                self.open_trade(row, &event, target_line as usize);
            }
            self.events.push(event);
        }
    }

    pub(crate) fn finish(&mut self, row: &CandleRow) {
        self.trades.finish(row);
    }

    fn open_trade(&mut self, row: &CandleRow, event: &CrossingEvent, target_line: usize) {
        let stop_line = if event.direction > 0 {
            event.line.checked_sub(1)
        } else {
            event.line.checked_add(1).filter(|line| *line < LINE_COUNT)
        };
        let Some(stop_line) = stop_line else {
            return;
        };
        let lines = row.mrc.lines();
        let entry_price = lines[event.line];
        let target_price = lines[target_line];
        let stop_price = lines[stop_line];
        if !mrc_crossing::valid_price_levels(
            event.direction,
            entry_price,
            target_price,
            stop_price,
            None,
        ) {
            return;
        }
        self.trades.queue_next_candle(TradeSetup::with_levels(
            event.direction,
            event.line,
            event.line_name,
            target_line,
            LABELS[target_line],
            entry_price,
            target_price,
            stop_price,
            row.time_ms,
            "three-line-mrc-crossing-pattern",
        ));
    }

    pub(crate) fn report(&self) -> AnalysisReport {
        let latest_event = self.events.last().copied();
        let patterns = pattern_reports(&self.model);
        let forecast = forecast(latest_event, &self.model);
        AnalysisReport {
            crossing_count: self.events.len(),
            pattern_count: patterns.iter().map(|item| item.count).sum(),
            patterns,
            latest_event,
            forecast,
            events: self.events.clone(),
            trade_summary: self.trades.summary(),
            trades: self.trades.completed(),
            open_trade: self.trades.active(),
        }
    }
}

fn pattern_report(
    pattern: [usize; MRC_PATTERN_LENGTH],
    to: usize,
    stats: long_scalping::state::MrcForecastStats,
    total: usize,
) -> PatternReport {
    PatternReport {
        pattern,
        pattern_names: std::array::from_fn(|index| LABELS[pattern[index]]),
        to,
        to_name: LABELS[to],
        count: stats.count as usize,
        probability: stats.count as f64 / total as f64,
        average_duration_ms: stats.average_duration_ms(),
    }
}

fn pattern_reports(model: &MrcCrossingModel) -> Vec<PatternReport> {
    let mut reports = Vec::new();
    for first in 0..LINE_COUNT {
        for second in 0..LINE_COUNT {
            for third in 0..LINE_COUNT {
                let stats = &model.patterns[first][second][third];
                let total = stats.observations() as usize;
                if total == 0 {
                    continue;
                }
                for to in 0..LINE_COUNT {
                    let next = stats.next[to];
                    if next.count > 0 {
                        reports.push(pattern_report([first, second, third], to, next, total));
                    }
                }
            }
        }
    }
    reports
}

fn forecast(latest: Option<CrossingEvent>, model: &MrcCrossingModel) -> Forecast {
    let Some(latest) = latest else {
        return Forecast {
            source_line: None,
            source_name: None,
            condition: "no-crossing-history",
            sample_count: 0,
            probability_next_crossing: None,
            estimated_time_ms: None,
            likely_destinations: Vec::new(),
        };
    };
    if model.pattern_len < MRC_PATTERN_LENGTH {
        return Forecast {
            source_line: Some(latest.line),
            source_name: Some(latest.line_name),
            condition: "three-line-pattern-warmup",
            sample_count: 0,
            probability_next_crossing: None,
            estimated_time_ms: None,
            likely_destinations: Vec::new(),
        };
    }
    let pattern = model.pattern_lines;
    let stats = &model.patterns[pattern[0] as usize][pattern[1] as usize][pattern[2] as usize];
    let total = stats.observations() as usize;
    let directional_total = stats
        .next
        .iter()
        .enumerate()
        .filter(|(to, _)| {
            if latest.direction > 0 {
                *to > latest.line
            } else {
                *to < latest.line
            }
        })
        .map(|(_, item)| item.count as usize)
        .sum::<usize>();
    let likely_destinations = stats
        .next
        .iter()
        .copied()
        .enumerate()
        .filter(|(to, item)| {
            item.count > 0
                && if latest.direction > 0 {
                    *to > latest.line
                } else {
                    *to < latest.line
                }
        })
        .map(|(to, item)| {
            pattern_report(
                [
                    pattern[0] as usize,
                    pattern[1] as usize,
                    pattern[2] as usize,
                ],
                to,
                item,
                total,
            )
        })
        .collect::<Vec<_>>();
    let estimated_time_ms = (directional_total > 0).then(|| {
        stats
            .next
            .iter()
            .enumerate()
            .filter(|(to, _)| {
                if latest.direction > 0 {
                    *to > latest.line
                } else {
                    *to < latest.line
                }
            })
            .map(|(_, item)| item.duration_sum_ms)
            .sum::<u64>() as f64
            / directional_total as f64
    });
    Forecast {
        source_line: Some(latest.line),
        source_name: Some(latest.line_name),
        condition: "three-line-mrc-crossing-pattern",
        sample_count: total,
        probability_next_crossing: (total > 0).then(|| directional_total as f64 / total as f64),
        estimated_time_ms,
        likely_destinations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use long_scalping::model::MrcBands;

    fn row(time_ms: i64, close: f64, high: f64, low: f64, ntps: i64) -> CandleRow {
        CandleRow {
            time_ms,
            open: close,
            high,
            low,
            close,
            zz_dir: 0,
            zz_break_price: 0.0,
            gain: 0.0,
            buy_sell_gain: 1.0,
            ntps,
            mrc: MrcBands {
                down_big: 90.0,
                down_outer: 92.0,
                down_small: 94.0,
                down_inner: 96.0,
                mean_line: 100.0,
                up_inner: 104.0,
                up_small: 106.0,
                up_outer: 108.0,
                up_big: 110.0,
                current_cross: 0.0,
                mean_line_slope: 0.0,
            },
        }
    }

    #[test]
    fn wick_crossings_are_ordered_and_recorded() {
        let mut analyzer = Analyzer::new(3);
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[1, 10]);
        analyzer.observe(&row(60_000, 105.0, 105.0, 99.0, 10), &[1, 10]);
        let report = analyzer.report();
        assert_eq!(report.crossing_count, 2);
        assert_eq!(report.events[0].line, 4);
        assert_eq!(report.events[1].line, 5);
    }

    #[test]
    fn three_crossings_are_warmup_before_a_forecast() {
        let mut analyzer = Analyzer::new(3);
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[]);
        analyzer.observe(&row(60_000, 101.0, 101.0, 99.0, 10), &[]);
        analyzer.observe(&row(120_000, 105.0, 105.0, 101.0, 10), &[]);
        analyzer.observe(&row(180_000, 107.0, 107.0, 105.0, 10), &[]);
        let report = analyzer.report();
        assert_eq!(report.forecast.condition, "three-line-mrc-crossing-pattern");
        assert_eq!(report.forecast.sample_count, 0);
        assert!(report.forecast.likely_destinations.is_empty());
    }
}
