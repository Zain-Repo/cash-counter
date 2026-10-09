use cash_counter::{
    company::{CompanyProfile, CompanyService, SetupCompany},
    database::{Database, Direction, NewEntry},
    reporting::{ReportFilter, report},
};

#[test]
fn company_setup_preserves_existing_ledger_and_reporting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.sqlite3");
    let mut ledger = Database::open(&path).unwrap();
    let register = ledger.create_register("Existing desk", "CAD").unwrap();
    let session = ledger.open_session(register, "2026-10-09", 10_000).unwrap();
    ledger
        .record_entries(
            session,
            &[NewEntry {
                direction: Direction::In,
                amount: 1_200,
                reason: "Existing sale",
                request_key: "existing-sale",
            }],
        )
        .unwrap();
    let mut company = CompanyService::open(&path).unwrap();
    company
        .setup(SetupCompany {
            profile: CompanyProfile {
                name: "Company using existing ledger".into(),
                ..Default::default()
            },
            username: "administrator".into(),
            password: "isolated test password".into(),
        })
        .unwrap();
    drop(company);
    drop(ledger);

    let ledger = Database::open(&path).unwrap();
    assert_eq!(ledger.balance(session).unwrap().expected, 11_200);
    let report = report(
        &ledger,
        &ReportFilter {
            start_date: "2026-10-09".into(),
            end_date: "2026-10-09".into(),
            register_id: Some(register),
        },
    )
    .unwrap();
    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.movements.len(), 1);
    assert_eq!(report.movements[0].reason, "Existing sale");
    assert!(
        CompanyService::open(path)
            .unwrap()
            .authenticate("administrator", "isolated test password")
            .unwrap()
    );
}
