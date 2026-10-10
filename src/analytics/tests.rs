use super::*;
use crate::database::{Database, Direction, NewEntry};

fn filter() -> DashboardFilter {
    DashboardFilter {
        start_date: "2026-10-01".into(),
        end_date: "2026-10-03".into(),
        currency: "CAD".into(),
        register_id: None,
    }
}

#[test]
fn dashboard_aggregates_and_zero_fills_without_mixing_currencies() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.db");
    let mut db = Database::open(&path).unwrap();
    let cad = db.create_register("Front", "CAD").unwrap();
    let usd = db.create_register("Other", "USD").unwrap();
    let session = db.open_session(cad, "2026-10-01", 1_000).unwrap();
    db.record_entries(
        session,
        &[
            NewEntry {
                direction: Direction::In,
                amount: 500,
                reason: "Sale",
                request_key: "in",
            },
            NewEntry {
                direction: Direction::Out,
                amount: 200,
                reason: "Refund",
                request_key: "out",
            },
        ],
    )
    .unwrap();
    db.close_session(session, 1_290).unwrap();
    db.open_session(cad, "2026-10-03", 1_000).unwrap();
    let other = db.open_session(usd, "2026-10-01", 0).unwrap();
    db.record_entries(
        other,
        &[NewEntry {
            direction: Direction::In,
            amount: 9999,
            reason: "Other",
            request_key: "usd",
        }],
    )
    .unwrap();
    let snapshot = load_dashboard(&path, &filter()).unwrap();
    assert_eq!(
        (
            snapshot.summary.sessions,
            snapshot.summary.open_sessions,
            snapshot.summary.closed_sessions
        ),
        (2, 1, 1)
    );
    assert_eq!(
        (
            snapshot.summary.cash_in,
            snapshot.summary.cash_out,
            snapshot.summary.net_flow,
            snapshot.summary.movement_count
        ),
        (500, 200, 300, 2)
    );
    assert_eq!(
        (
            snapshot.summary.discrepancy_count,
            snapshot.summary.discrepancy_total
        ),
        (1, -10)
    );
    assert_eq!(snapshot.daily_flow.len(), 3);
    assert_eq!(
        (
            snapshot.daily_flow[1].cash_in,
            snapshot.daily_flow[1].movement_count
        ),
        (0, 0)
    );
    assert_eq!(snapshot.discrepancies[0].session_id, session);
    assert_eq!(snapshot.register_rankings[0].register_id, cad);
}

#[test]
fn empty_dashboard_returns_zero_totals_and_bounded_days() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.db");
    Database::open(&path).unwrap();
    let snapshot = load_dashboard(&path, &filter()).unwrap();
    assert_eq!(snapshot.summary.sessions, 0);
    assert_eq!(snapshot.daily_flow.len(), 3);
    assert!(snapshot.register_rankings.is_empty());
}

#[test]
fn invalid_filters_are_rejected_before_opening_database() {
    for (start, end, currency, id) in [
        ("2026-02-30", "2026-03-01", "CAD", None),
        ("2026-10-03", "2026-10-01", "CAD", None),
        ("2025-01-01", "2026-01-02", "CAD", None),
        ("2026-10-01", "2026-10-03", "cad", None),
        ("2026-10-01", "2026-10-03", "CAD", Some(0)),
    ] {
        let selection = DashboardFilter {
            start_date: start.into(),
            end_date: end.into(),
            currency: currency.into(),
            register_id: id,
        };
        assert!(matches!(
            load_dashboard(Path::new("missing.db"), &selection),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn overflow_fails_instead_of_returning_approximate_money() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.db");
    let mut db = Database::open(&path).unwrap();
    for index in 0..2 {
        let register = db
            .create_register(&format!("Register {index}"), "CAD")
            .unwrap();
        let session = db.open_session(register, "2026-10-01", 0).unwrap();
        db.record_entries(
            session,
            &[NewEntry {
                direction: Direction::In,
                amount: i64::MAX,
                reason: "Large",
                request_key: &format!("large-{index}"),
            }],
        )
        .unwrap();
    }
    assert!(matches!(
        load_dashboard(&path, &filter()),
        Err(Error::Database(_))
    ));
}

#[test]
fn discrepancy_ranking_preserves_integer_precision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.db");
    let mut db = Database::open(&path).unwrap();
    let mut largest_session = 0;
    for (index, difference) in [9_007_199_254_740_993_i64, 9_007_199_254_740_992]
        .into_iter()
        .enumerate()
    {
        let register = db
            .create_register(&format!("Register {index}"), "CAD")
            .unwrap();
        let session = db.open_session(register, "2026-10-01", 0).unwrap();
        db.close_session(session, difference).unwrap();
        if index == 0 {
            largest_session = session;
        }
    }
    let snapshot = load_dashboard(&path, &filter()).unwrap();
    assert_eq!(snapshot.discrepancies[0].session_id, largest_session);
}

#[test]
fn selected_register_isolated_and_detail_lists_are_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.db");
    let mut db = Database::open(&path).unwrap();
    let mut selected_register = 0;
    for index in 0..25 {
        let register = db
            .create_register(&format!("Register {index}"), "CAD")
            .unwrap();
        selected_register = register;
        let session = db.open_session(register, "2026-10-01", 100).unwrap();
        db.close_session(session, 100 + index + 1).unwrap();
    }
    let all = load_dashboard(&path, &filter()).unwrap();
    assert_eq!(all.summary.discrepancy_count, 25);
    assert_eq!(all.discrepancies.len(), 20);
    assert_eq!(all.discrepancies[0].difference, 25);
    assert_eq!(all.register_rankings.len(), 10);
    let mut selection = filter();
    selection.register_id = Some(selected_register);
    let selected = load_dashboard(&path, &selection).unwrap();
    assert_eq!(selected.summary.sessions, 1);
    assert_eq!(selected.summary.discrepancy_total, 25);
}
