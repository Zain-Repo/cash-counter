use cash_counter::{
    backup::backup_database,
    backup_service::{BackupFrequency, BackupMode, BackupService, BackupSettings},
    company::{CompanyProfile, CompanyService, SetupCompany},
    database::{Database, Direction, NewEntry},
    restore::{resolve_backup, restore_database},
};
use rusqlite::{Connection, TransactionBehavior};
use std::{path::Path, time::Duration};

fn ledger(path: &Path, amount: i64) {
    let mut database = Database::open(path).unwrap();
    let register = database.create_register("Front desk", "CAD").unwrap();
    let session = database
        .open_session(register, "2026-10-08", 10_000)
        .unwrap();
    database
        .record_entries(
            session,
            &[NewEntry {
                direction: Direction::In,
                amount,
                reason: "Sale",
                request_key: "sale-1",
            }],
        )
        .unwrap();
    drop(database);
    let mut company = CompanyService::open(path).unwrap();
    company
        .setup(SetupCompany {
            profile: CompanyProfile {
                name: "Example Company".into(),
                ..Default::default()
            },
            username: "admin".into(),
            password: "correct horse battery".into(),
        })
        .unwrap();
}

fn scalar(path: &Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn restore_recovers_ledger_and_keeps_current_backup_policy() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    let original = directory.path().join("original.sqlite");
    let backup = directory
        .path()
        .join("cash-counter-20261008T000000Z-test.sqlite");
    ledger(&original, 500);
    backup_database(&original, &backup).unwrap();
    ledger(&live, 900);
    let mut backups = BackupService::open(&live).unwrap();
    let settings = BackupSettings {
        directory: Some(directory.path().to_path_buf()),
        mode: BackupMode::Automatic,
        frequency: BackupFrequency::Hourly,
    };
    let saved_settings = backups
        .save_settings(&settings, 1_800_000_000)
        .unwrap()
        .settings;
    drop(backups);
    Connection::open(&live).unwrap().execute(
        "UPDATE backup_service_state SET last_success_at=123,last_backup_path='prior.sqlite',last_error='stale',lease_token='stale',lease_until=1 WHERE id=1", [],
    ).unwrap();

    let summary = restore_database(&live, &backup).unwrap();
    assert_eq!(scalar(&live, "SELECT amount FROM cash_in"), 500);
    assert!(summary.safety_backup.is_file());
    assert_eq!(
        scalar(&summary.safety_backup, "SELECT amount FROM cash_in"),
        900
    );
    let service = BackupService::open(&live).unwrap();
    let status = service.status().unwrap();
    assert_eq!(status.settings, saved_settings);
    assert_eq!(status.last_success_at, Some(123));
    assert_eq!(
        status.last_backup_path.as_deref(),
        Some(Path::new("prior.sqlite"))
    );
    assert!(status.next_due_at.unwrap() > chrono::Utc::now().timestamp());
    assert!(status.last_error.is_none());
    assert_eq!(
        scalar(
            &live,
            "SELECT COUNT(*) FROM backup_service_state WHERE lease_token IS NULL AND lease_until IS NULL"
        ),
        1
    );
    assert_eq!(scalar(&backup, "SELECT amount FROM cash_in"), 500);
}

#[test]
fn invalid_backup_and_live_file_are_rejected_without_changing_live_data() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    ledger(&live, 700);
    let arbitrary = directory.path().join("ordinary.sqlite");
    Connection::open(&arbitrary)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES('keep');")
        .unwrap();
    assert!(restore_database(&live, &arbitrary).is_err());
    assert!(restore_database(&live, &live).is_err());
    assert!(resolve_backup(&live, &live).is_err());
    assert_eq!(scalar(&live, "SELECT amount FROM cash_in"), 700);
    assert_eq!(
        Connection::open(&arbitrary)
            .unwrap()
            .query_row("SELECT value FROM unrelated", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn restore_fails_cleanly_when_another_connection_holds_a_read_lock() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    let original = directory.path().join("original.sqlite");
    let backup = directory
        .path()
        .join("cash-counter-20261008T000000Z-test.sqlite");
    ledger(&original, 300);
    backup_database(&original, &backup).unwrap();
    ledger(&live, 600);

    let reader = Connection::open(&live).unwrap();
    reader.busy_timeout(Duration::from_millis(100)).unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&reader, TransactionBehavior::Deferred).unwrap();
    let _: i64 = tx
        .query_row("SELECT COUNT(*) FROM cash_in", [], |row| row.get(0))
        .unwrap();
    assert!(restore_database(&live, &backup).is_err());
    drop(tx);
    drop(reader);
    assert_eq!(scalar(&live, "SELECT amount FROM cash_in"), 600);
}

#[test]
fn folder_selection_uses_newest_generated_backup_and_skips_recovery_snapshots() {
    use std::time::SystemTime;

    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    ledger(&live, 100);
    let older = directory
        .path()
        .join("cash-counter-20261001T000000Z-a.sqlite");
    let newer = directory
        .path()
        .join("cash-counter-20261002T000000Z-b.sqlite");
    let recovery = directory.path().join("cash-counter-pre-restore-old.sqlite");
    for path in [&older, &newer, &recovery] {
        backup_database(&live, path).unwrap();
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(&older)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(30)),
        )
        .unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&newer)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()))
        .unwrap();
    assert_eq!(
        resolve_backup(directory.path(), &live).unwrap(),
        newer.canonicalize().unwrap()
    );
}

#[test]
fn corrupt_and_future_schema_backups_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    let backup = directory.path().join("cash-counter-valid.sqlite");
    let corrupt = directory.path().join("corrupt.sqlite");
    ledger(&live, 200);
    backup_database(&live, &backup).unwrap();
    std::fs::write(&corrupt, b"not sqlite").unwrap();
    assert!(restore_database(&live, &corrupt).is_err());

    let future = directory.path().join("future.sqlite");
    backup_database(&live, &future).unwrap();
    Connection::open(&future)
        .unwrap()
        .pragma_update(None, "user_version", 4)
        .unwrap();
    assert!(restore_database(&live, &future).is_err());
    assert_eq!(scalar(&live, "SELECT amount FROM cash_in"), 200);
}

#[test]
fn active_backup_lease_prevents_restore_before_safety_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    let original = directory.path().join("original.sqlite");
    let backup = directory
        .path()
        .join("cash-counter-20261008T000000Z-test.sqlite");
    ledger(&original, 100);
    backup_database(&original, &backup).unwrap();
    ledger(&live, 400);
    drop(BackupService::open(&live).unwrap());
    Connection::open(&live).unwrap().execute(
        "UPDATE backup_service_state SET lease_token='active',lease_until=9999999999 WHERE id=1", [],
    ).unwrap();
    assert!(restore_database(&live, &backup).is_err());
    assert_eq!(scalar(&live, "SELECT amount FROM cash_in"), 400);
}

#[test]
fn incomplete_credentials_are_rejected_without_replacing_the_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live.sqlite");
    let backup = directory.path().join("incomplete-credentials.sqlite");
    ledger(&live, 800);
    backup_database(&live, &backup).unwrap();
    Connection::open(&backup)
        .unwrap()
        .execute(
            "UPDATE company_users SET password_hash = ?1",
            ["$argon2id$v=19$m=19456,t=2,p=1"],
        )
        .unwrap();
    assert!(restore_database(&live, &backup).is_err());
    assert_eq!(scalar(&live, "SELECT amount FROM cash_in"), 800);
    assert!(
        CompanyService::open(&live)
            .unwrap()
            .authenticate("admin", "correct horse battery")
            .unwrap()
    );
}
