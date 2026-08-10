use long_scalping::detection::{mrc_reclaim, vlines};
use long_scalping::state::{MrcCrossingState, MrcReclaimState, VLineState};
use serde::Serialize;

use super::data::CandleRow;
use super::virtual_trade::{TradeSetup, VirtualTradeSimulator};
pub(crate) type TradeResult = super::virtual_trade::VirtualTradeResult;
pub(crate) type TradeSummary = super::virtual_trade::VirtualTradeSummary;
pub(crate) type VirtualPosition = super::virtual_trade::VirtualPosition;

const LINE_COUNT: usize = 9;
pub(crate) const BREAK_MS: i64 = mrc_reclaim::BREAK_LINE_MS as i64;
const LABELS: [&str; LINE_COUNT] = mrc_reclaim::LINE_LABELS;

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CrossingEvent {
    pub(crate) time_ms: i64,
    pub(crate) line: usize,
    pub(crate) line_name: &'static str,
    pub(crate) direction: i8,
    pub(crate) price: f64,
    pub(crate) ntps: i64,
    pub(crate) ntps_min: Option<i64>,
    pub(crate) ntps_max: Option<i64>,
    pub(crate) small_ntps: bool,
    pub(crate) recent_nearest_crossings: usize,
    pub(crate) frequent_nearest_crossings: bool,
    pub(crate) break_line: bool,
    pub(crate) mode: &'static str,
    pub(crate) buy_volume: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransitionReport {
    pub(crate) from: usize,
    pub(crate) from_name: &'static str,
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
    pub(crate) likely_destinations: Vec<TransitionReport>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnalysisReport {
    pub(crate) crossing_count: usize,
    pub(crate) transition_count: usize,
    pub(crate) break_event_count: usize,
    pub(crate) scenario_event_count: usize,
    pub(crate) transitions: Vec<TransitionReport>,
    pub(crate) scenario_transitions: Vec<TransitionReport>,
    pub(crate) latest_event: Option<CrossingEvent>,
    pub(crate) forecast: Forecast,
    pub(crate) events: Vec<CrossingEvent>,
    pub(crate) trade_summary: TradeSummary,
    pub(crate) trades: Vec<TradeResult>,
    pub(crate) open_trade: Option<VirtualPosition>,
    pub(crate) wait_mode: bool,
    pub(crate) wait_direction: i8,
}

#[derive(Debug, Clone)]
pub(crate) struct Analyzer {
    vlines: [VLineState; LINE_COUNT],
    crossing: MrcCrossingState,
    reclaim: MrcReclaimState,
    events: Vec<CrossingEvent>,
    trades: VirtualTradeSimulator,
    wait_mode: bool,
    wait_direction: i8,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self {
            vlines: [VLineState::default(); LINE_COUNT],
            crossing: MrcCrossingState::default(),
            reclaim: MrcReclaimState::default(),
            events: Vec::new(),
            trades: VirtualTradeSimulator::default(),
            wait_mode: false,
            wait_direction: 0,
        }
    }
}

impl Analyzer {
    pub(crate) fn observe(&mut self, row: &CandleRow, hourly_ntps: &[i64]) {
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
        let ntps_range = hourly_ntps
            .iter()
            .copied()
            .min()
            .zip(hourly_ntps.iter().copied().max());
        let observation = mrc_reclaim::observe_with_context(
            &mut self.reclaim,
            &crossed,
            lines,
            row.close,
            row.time_ms.max(0) as u64,
            row.buy_sell_gain,
            Some((row.ntps, ntps_range)),
        );
        for event in observation.events {
            let target_line = event.target_line;
            let event = CrossingEvent {
                time_ms: event.now_ms as i64,
                line: event.line_idx as usize,
                line_name: LABELS[event.line_idx as usize],
                direction: event.direction,
                price: event.line,
                ntps: row.ntps,
                ntps_min: ntps_range.map(|(min, _)| min),
                ntps_max: ntps_range.map(|(_, max)| max),
                small_ntps: event.small_ntps,
                recent_nearest_crossings: event.recent_nearest_crossings,
                frequent_nearest_crossings: event.frequent_nearest_crossings,
                break_line: event.break_line,
                mode: if event.break_line { "wait" } else { "normal" },
                buy_volume: event.buy_volume,
            };
            if event.break_line {
                self.trades
                    .close_active(row.close, row.time_ms, "break-line", "market");
            } else if let Some(target_line) = target_line {
                self.open_trade(row, &event, target_line as usize);
            }
            self.events.push(event);
        }
        self.wait_mode = self.reclaim.wait_direction != 0;
        self.wait_direction = self.reclaim.wait_direction;
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
        if !mrc_reclaim::valid_price_levels(
            event.direction,
            entry_price,
            target_price,
            stop_price,
            None,
        ) {
            return;
        }
        self.trades.open(TradeSetup::with_levels(
            event.direction,
            event.line,
            event.line_name,
            target_line,
            LABELS[target_line],
            entry_price,
            target_price,
            stop_price,
            row.time_ms,
            if event.small_ntps && event.frequent_nearest_crossings {
                "small-ntps-frequent-crossings"
            } else {
                "historical-next-line"
            },
        ));
    }

    pub(crate) fn report(&self) -> AnalysisReport {
        let transitions = matrix_reports(&self.reclaim.transitions);
        let scenario_transitions = matrix_reports(&self.reclaim.scenario_transitions);
        let latest_event = self.events.last().copied();
        let use_scenario =
            latest_event.is_some_and(|event| event.small_ntps && event.frequent_nearest_crossings);
        let source_observations = if use_scenario {
            &self.reclaim.scenario_source_counts
        } else {
            &self.reclaim.source_counts
        };
        let has_scenario_history =
            latest_event.is_some_and(|event| self.reclaim.scenario_source_counts[event.line] > 0);
        let use_scenario = use_scenario && has_scenario_history;
        let forecast = forecast(
            latest_event,
            if use_scenario {
                &self.reclaim.scenario_transitions
            } else {
                &self.reclaim.transitions
            },
            use_scenario,
            source_observations,
        );
        AnalysisReport {
            crossing_count: self.events.len(),
            transition_count: transitions.iter().map(|item| item.count).sum(),
            break_event_count: self.events.iter().filter(|event| event.break_line).count(),
            scenario_event_count: self
                .events
                .iter()
                .filter(|event| event.small_ntps && event.frequent_nearest_crossings)
                .count(),
            transitions,
            scenario_transitions,
            latest_event,
            forecast,
            events: self.events.clone(),
            trade_summary: self.trades.summary(),
            trades: self.trades.completed(),
            open_trade: self.trades.active(),
            wait_mode: self.reclaim.wait_direction != 0,
            wait_direction: self.reclaim.wait_direction,
        }
    }
}
fn matrix_reports(
    matrix: &[[long_scalping::state::MrcTransitionStats; LINE_COUNT]; LINE_COUNT],
) -> Vec<TransitionReport> {
    let mut reports = Vec::new();
    for from in 0..LINE_COUNT {
        let total = matrix[from]
            .iter()
            .map(|stats| stats.count as usize)
            .sum::<usize>();
        if total == 0 {
            continue;
        }
        for to in 0..LINE_COUNT {
            let stats = matrix[from][to];
            if stats.count == 0 {
                continue;
            }
            reports.push(TransitionReport {
                from,
                from_name: LABELS[from],
                to,
                to_name: LABELS[to],
                count: stats.count as usize,
                probability: stats.count as f64 / total as f64,
                average_duration_ms: stats.average_duration_ms(),
            });
        }
    }
    reports
}

fn forecast(
    latest: Option<CrossingEvent>,
    matrix: &[[long_scalping::state::MrcTransitionStats; LINE_COUNT]; LINE_COUNT],
    scenario: bool,
    source_observations: &[u32; LINE_COUNT],
) -> Forecast {
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
    let stats = matrix[latest.line];
    let sample_count = stats.iter().map(|item| item.count as usize).sum::<usize>();
    let source_count = source_observations[latest.line];
    let average = (sample_count > 0).then(|| {
        stats.iter().map(|item| item.duration_sum_ms).sum::<u64>() as f64 / sample_count as f64
    });
    Forecast {
        source_line: Some(latest.line),
        source_name: Some(latest.line_name),
        condition: if scenario {
            "small-ntps-and-frequent-nearest-crossings"
        } else {
            "all-history-for-source-line"
        },
        sample_count,
        probability_next_crossing: (source_count > 0)
            .then(|| sample_count as f64 / source_count as f64),
        estimated_time_ms: average,
        likely_destinations: matrix_reports_for_source(latest.line, matrix),
    }
}

fn matrix_reports_for_source(
    from: usize,
    matrix: &[[long_scalping::state::MrcTransitionStats; LINE_COUNT]; LINE_COUNT],
) -> Vec<TransitionReport> {
    let total = matrix[from]
        .iter()
        .map(|stats| stats.count as usize)
        .sum::<usize>();
    if total == 0 {
        return Vec::new();
    }
    (0..LINE_COUNT)
        .filter_map(|to| {
            let stats = matrix[from][to];
            (stats.count > 0).then(|| TransitionReport {
                from,
                from_name: LABELS[from],
                to,
                to_name: LABELS[to],
                count: stats.count as usize,
                probability: stats.count as f64 / total as f64,
                average_duration_ms: stats.average_duration_ms(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::virtual_trade::FeeModel;
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
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[1, 10]);
        analyzer.observe(&row(60_000, 105.0, 105.0, 99.0, 10), &[1, 10]);
        let report = analyzer.report();
        assert_eq!(report.crossing_count, 2);
        assert_eq!(report.events[0].line, 4);
        assert_eq!(report.events[1].line, 5);
    }

    #[test]
    fn transitions_have_probability_and_duration() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[1, 10]);
        analyzer.observe(&row(60_000, 101.0, 101.0, 99.0, 10), &[1, 10]);
        analyzer.observe(&row(180_000, 105.0, 105.0, 101.0, 10), &[1, 10]);
        let report = analyzer.report();
        let transition = report
            .transitions
            .iter()
            .find(|item| item.from == 4 && item.to == 5)
            .unwrap();
        assert_eq!(transition.count, 1);
        assert_eq!(transition.average_duration_ms, Some(120_000.0));
        assert_eq!(transition.probability, 1.0);
    }

    #[test]
    fn low_ntps_uses_hourly_range_lower_third() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(60_000, 101.0, 101.0, 99.0, 30), &[10, 100]);
        assert!(analyzer.report().events[0].small_ntps);
    }

    #[test]
    fn stale_line_is_flagged_as_break_line() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(1_800_000, 101.0, 101.0, 99.0, 10), &[10, 100]);
        assert!(analyzer.report().events[0].break_line);
        assert_eq!(analyzer.report().events[0].mode, "wait");
    }

    #[test]
    fn frequent_nearest_crossings_are_detected() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(60_000, 105.0, 105.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(120_000, 107.0, 107.0, 105.0, 10), &[10, 100]);
        analyzer.observe(&row(180_000, 103.0, 107.0, 103.0, 10), &[10, 100]);
        let events = analyzer.report().events;
        assert!(events.iter().any(|event| event.frequent_nearest_crossings));
    }

    #[test]
    fn crossing_probability_uses_source_observation_count() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(60_000, 101.0, 101.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(120_000, 99.0, 101.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(180_000, 105.0, 105.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(240_000, 99.0, 105.0, 99.0, 100), &[10, 100]);
        let probability = analyzer.report().forecast.probability_next_crossing;
        assert_eq!(probability, Some(0.5));
    }

    #[test]
    fn learned_next_line_opens_virtual_trade_and_hits_target() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(60_000, 101.0, 101.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(120_000, 105.0, 105.0, 101.0, 100), &[10, 100]);
        analyzer.observe(&row(180_000, 99.0, 105.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(240_000, 105.0, 105.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(300_000, 99.0, 105.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(360_000, 105.0, 105.0, 99.0, 100), &[10, 100]);
        analyzer.observe(&row(420_000, 95.0, 105.0, 95.0, 100), &[10, 100]);
        analyzer.observe(&row(480_000, 101.0, 101.0, 95.0, 100), &[10, 100]);
        analyzer.observe(&row(540_000, 103.0, 105.0, 101.0, 100), &[10, 100]);
        let report = analyzer.report();
        assert_eq!(report.trade_summary.trades, 1);
        assert_eq!(report.trades[0].source_name, "mean");
        assert_eq!(report.trades[0].target_name, "up_inner");
        assert_eq!(report.trades[0].exit_reason, "target");
        assert!((report.trades[0].gross_pnl_pct - 4.0).abs() < 1e-12);
        assert_eq!(
            report.trades[0].entry_fee_pct,
            FeeModel::default().limit_fee_pct
        );
        assert!((report.trades[0].exit_fee_pct - 0.0208).abs() < 1e-12);
        assert!((report.trades[0].pnl_pct - 3.9592).abs() < 1e-12);
        assert!((report.trade_summary.total_fees_pct - 0.0408).abs() < 1e-12);
        assert_eq!(report.trade_summary.max_drawdown_pct, 0.0);
        assert!(report.trades[0].pnl_pct > 0.0);
    }

    #[test]
    fn break_line_enters_wait_mode_without_opening_trade() {
        let mut analyzer = Analyzer::default();
        analyzer.observe(&row(0, 99.0, 99.0, 99.0, 10), &[10, 100]);
        analyzer.observe(&row(BREAK_MS, 101.0, 101.0, 99.0, 10), &[10, 100]);
        let report = analyzer.report();
        assert_eq!(report.trade_summary.trades, 0);
        assert!(report.wait_mode);
        assert_eq!(report.wait_direction, 1);
    }
}
