use long_scalping::model::{Candle, MrcBands, PositionState};
use long_scalping::strategies::trending::{entry, exit};

fn bands() -> MrcBands {
    MrcBands {
        down_big: 90.0,
        down_outer: 92.0,
        down_small: 94.0,
        down_inner: 96.0,
        mean_line: 100.0,
        up_inner: 104.0,
        up_small: 106.0,
        up_outer: 108.0,
        up_big: 110.0,
        ..MrcBands::default()
    }
}

#[test]
fn long_entry_uses_down_big_as_structural_stop() {
    let decision = entry::evaluate(
        None,
        Some(1),
        bands(),
        Candle {
            open: 99.0,
            high: 103.0,
            low: 98.0,
            close: 103.0,
        },
        101.0,
    );

    let entry::Decision::Enter(setup) = decision else {
        panic!("expected a long trend entry, got {decision:?}");
    };
    assert_eq!(setup.stop_line, 0);
    assert_eq!(setup.stop_loss, 90.0);
}

#[test]
fn short_entry_uses_up_big_as_structural_stop() {
    let decision = entry::evaluate(
        None,
        Some(-1),
        bands(),
        Candle {
            open: 101.0,
            high: 102.0,
            low: 97.0,
            close: 97.0,
        },
        99.0,
    );

    let entry::Decision::Enter(setup) = decision else {
        panic!("expected a short trend entry, got {decision:?}");
    };
    assert_eq!(setup.stop_line, 8);
    assert_eq!(setup.stop_loss, 110.0);
}

#[test]
fn long_stop_stays_at_down_big_until_trailing_line_is_above_entry() {
    let current = exit::evaluate(PositionState::Long, 101.0, 102.0, 90.0, bands());
    assert_eq!(current, exit::Decision::Hold);

    let favorable = exit::evaluate(PositionState::Long, 101.0, 112.0, 90.0, bands());
    assert_eq!(
        favorable,
        exit::Decision::Update(exit::Update {
            stop_loss: Some(108.0),
            reason: "mrc-trailing-stop",
        })
    );
}

#[test]
fn long_profit_stop_is_preserved_across_later_mrc_updates() {
    let decision = exit::evaluate(PositionState::Long, 101.0, 102.0, 105.0, bands());
    assert_eq!(decision, exit::Decision::Hold);
}

#[test]
fn short_stop_stays_at_up_big_until_trailing_line_is_below_entry() {
    let current = exit::evaluate(PositionState::Short, 99.0, 98.0, 110.0, bands());
    assert_eq!(current, exit::Decision::Hold);

    let favorable = exit::evaluate(PositionState::Short, 99.0, 88.0, 110.0, bands());
    assert_eq!(
        favorable,
        exit::Decision::Update(exit::Update {
            stop_loss: Some(92.0),
            reason: "mrc-trailing-stop",
        })
    );
}

#[test]
fn long_sideways_handoff_keeps_structural_stop_until_candidate_is_profitable() {
    assert_eq!(
        exit::protect_handoff_stop_loss(PositionState::Long, 101.0, 90.0, 96.0, bands()),
        Some((90.0, false))
    );

    assert_eq!(
        exit::protect_handoff_stop_loss(PositionState::Long, 101.0, 90.0, 104.0, bands()),
        Some((104.0, true))
    );
}

#[test]
fn short_sideways_handoff_keeps_structural_stop_until_candidate_is_profitable() {
    assert_eq!(
        exit::protect_handoff_stop_loss(PositionState::Short, 99.0, 110.0, 104.0, bands()),
        Some((110.0, false))
    );

    assert_eq!(
        exit::protect_handoff_stop_loss(PositionState::Short, 99.0, 110.0, 96.0, bands()),
        Some((96.0, true))
    );
}
