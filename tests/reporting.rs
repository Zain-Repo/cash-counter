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
    db.open_session(other, "2026-10-09", 500).unwrap();
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
    assert!(csv.contains("\"'=Danger, \"\"Desk\"\"\""));
    assert!(csv.contains("\"'\t=FORMULA\nquoted \"\"value\"\"\""));
    let printable = result.printable("Company\nName");
    assert!(printable.contains("Company Name"));
    assert!(printable.contains("Expected 11.50"));
    assert!(printable.contains("Difference -0.50"));
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
