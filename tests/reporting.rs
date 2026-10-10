use cash_counter::{
    database::{Database, Direction, NewEntry},
    reporting::{ReportFilter, report},
};

#[test]
fn date_filter_reconciliation_and_csv_safety() {
    let mut db = Database::in_memory().unwrap();
    let register = db.create_register("=Danger, \"Desk\"", "CAD").unwrap();
    let other = db.create_register("Other", "USD").unwrap();
    let session = db.open_session(register, "2026-10-09", 1000).unwrap();
    let other_session = db.open_session(other, "2026-10-09", 500).unwrap();
    db.record_entries(
        other_session,
        &[NewEntry {
            direction: Direction::In,
            amount: 900,
            reason: "Excluded register",
            request_key: "excluded-register",
        }],
    )
    .unwrap();
    db.record_entries(
        session,
        &[
            NewEntry {
                direction: Direction::In,
                amount: 250,
                reason: "\t=FORMULA\nquoted \"value\"",
                request_key: "report-in",
            },
            NewEntry {
                direction: Direction::Out,
                amount: 100,
                reason: "Petty cash",
                request_key: "report-out",
            },
        ],
    )
    .unwrap();
    db.close_session(session, 1100).unwrap();
    let filter = ReportFilter {
        start_date: "2026-10-09".into(),
        end_date: "2026-10-09".into(),
        register_id: Some(register),
    };
    let result = report(&db, &filter).unwrap();
    assert_eq!(result.sessions.len(), 1);
    assert_eq!(result.movements.len(), 2);
    let summary = &result.sessions[0];
    assert_eq!(
        (
            summary.cash_in,
            summary.cash_out,
            summary.expected,
            summary.difference
        ),
        (250, 100, 1150, Some(-50))
    );
    let csv = result.to_csv();
    let mut streamed = Vec::new();
    result.write_csv(&mut streamed).unwrap();
    assert_eq!(streamed, csv.as_bytes());
    assert!(csv.contains("\"'=Danger, \"\"Desk\"\"\""));
    assert!(csv.contains("\"'\t=FORMULA\nquoted \"\"value\"\"\""));
    let printable = result.printable("Company\nName");
    assert!(printable.contains("Company Name"));
    assert!(printable.contains("Expected 11.50"));
    assert!(printable.contains("Difference -0.50"));
}

#[test]
fn report_excludes_movements_outside_business_date_bounds() {
    let mut db = Database::in_memory().unwrap();
    let register = db.create_register("Desk", "CAD").unwrap();
    let earlier = db.open_session(register, "2026-10-08", 0).unwrap();
    db.record_entries(
        earlier,
        &[NewEntry {
            direction: Direction::In,
            amount: 100,
            reason: "Previous day",
            request_key: "earlier-day",
        }],
    )
    .unwrap();
    db.close_session(earlier, 100).unwrap();
    let current = db.open_session(register, "2026-10-09", 100).unwrap();
    db.record_entries(
        current,
        &[NewEntry {
            direction: Direction::Out,
            amount: 50,
            reason: "Current day",
            request_key: "current-day",
        }],
    )
    .unwrap();
    let result = report(
        &db,
        &ReportFilter {
            start_date: "2026-10-09".into(),
            end_date: "2026-10-09".into(),
            register_id: None,
        },
    )
    .unwrap();
    assert_eq!(
        result
            .movements
            .iter()
            .map(|m| m.session_id)
            .collect::<Vec<_>>(),
        vec![current]
    );
}

#[test]
fn streaming_csv_propagates_destination_errors() {
    struct FailedDestination;
    impl std::io::Write for FailedDestination {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::StorageFull.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let db = Database::in_memory().unwrap();
    let result = report(
        &db,
        &ReportFilter {
            start_date: "2026-10-09".into(),
            end_date: "2026-10-09".into(),
            register_id: None,
        },
    )
    .unwrap();
    assert_eq!(
        result.write_csv(&mut FailedDestination).unwrap_err().kind(),
        std::io::ErrorKind::StorageFull
    );
}

#[test]
fn invalid_bounds_are_rejected_and_empty_period_is_reported() {
    let db = Database::in_memory().unwrap();
    for (start, end) in [
        ("2026-02-30", "2026-03-01"),
        ("2026-10-10", "2026-10-09"),
        ("2026-1-01", "2026-10-09"),
    ] {
        assert!(
            report(
                &db,
                &ReportFilter {
                    start_date: start.into(),
                    end_date: end.into(),
                    register_id: None
                }
            )
            .is_err()
        );
    }
    let result = report(
        &db,
        &ReportFilter {
            start_date: "2026-10-09".into(),
            end_date: "2026-10-09".into(),
            register_id: None,
        },
    )
    .unwrap();
    assert!(result.sessions.is_empty());
    assert!(result.printable("Company").contains("No sessions"));
}

#[test]
fn export_preserves_databases_and_replaces_only_csv() {
    use cash_counter::reporting::export_csv;
    let directory = tempfile::tempdir().unwrap();
    let ledger = directory.path().join("ledger.csv");
    let db = Database::open(&ledger).unwrap();
    let result = report(
        &db,
        &ReportFilter {
            start_date: "2026-10-09".into(),
            end_date: "2026-10-09".into(),
            register_id: None,
        },
    )
    .unwrap();
    assert!(export_csv(&result, &ledger, &ledger).is_err());
    assert!(export_csv(&result, &directory.path().join("ledger.csv-wal"), &ledger).is_err());
    let other = directory.path().join("other.csv");
    std::fs::write(&other, b"SQLite format 3\0rest").unwrap();
    assert!(export_csv(&result, &other, &ledger).is_err());
    assert_eq!(std::fs::read(&other).unwrap(), b"SQLite format 3\0rest");
    let csv = directory.path().join("report.csv");
    std::fs::write(&csv, "old report").unwrap();
    export_csv(&result, &csv, &ledger).unwrap();
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), result.to_csv());
    assert!(
        !std::fs::read_dir(directory.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    assert!(db.registers().unwrap().is_empty());
}
