use std::path::PathBuf;

use cash_counter::{
    backup_service::{BackupFrequency, BackupMode, BackupService, BackupSettings},
    database::Database,
};
use rusqlite::Connection;

fn existing_ledger(directory: &std::path::Path) -> PathBuf {
    let path = directory.join("ledger.sqlite");
    drop(Database::open(&path).expect("create ledger"));
    path
}

#[test]
fn new_ledger_uses_manual_defaults_and_persists_preferences() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let mut service = BackupService::open(&ledger).expect("open service");
    let defaults = service.status().expect("read defaults");
    assert_eq!(defaults.settings.mode, BackupMode::Manual);
    assert_eq!(defaults.settings.frequency, BackupFrequency::Daily);
    assert_eq!(defaults.next_due_at, None);

    let settings = BackupSettings {
        directory: Some(directory.path().to_path_buf()),
        mode: BackupMode::Automatic,
        frequency: BackupFrequency::EverySixHours,
    };
    let saved = service
        .save_settings(&settings, 1_800_000_000)
        .expect("save settings");
    assert_eq!(
        saved.settings.directory,
        Some(directory.path().canonicalize().expect("canonical folder"))
    );
    assert_eq!(saved.next_due_at, Some(1_800_000_000));

    let reopened = BackupService::open(&ledger).expect("reopen service");
    assert_eq!(
        reopened.status().expect("read persisted settings").settings,
        settings_with_canonical_path(directory.path())
    );
}

fn settings_with_canonical_path(directory: &std::path::Path) -> BackupSettings {
    BackupSettings {
        directory: Some(directory.canonicalize().expect("canonical folder")),
        mode: BackupMode::Automatic,
        frequency: BackupFrequency::EverySixHours,
    }
}

#[test]
fn automatic_backup_runs_only_when_due_and_advances_the_schedule() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let mut service = BackupService::open(&ledger).expect("open service");
    service
        .save_settings(
            &BackupSettings {
                directory: Some(directory.path().to_path_buf()),
                mode: BackupMode::Automatic,
                frequency: BackupFrequency::Hourly,
            },
            100,
        )
        .expect("save settings");

    assert!(service.run_backup(true, 99).expect("not due").is_none());
    let backup = service
        .run_backup(true, 100)
        .expect("run due backup")
        .expect("backup ran");
    assert!(backup.path.is_file());
    let status = service.status().expect("read status");
    assert!(status.last_success_at.is_some_and(|time| time >= 100));
    assert_eq!(status.last_backup_path, Some(backup.path));
    assert_eq!(
        status.next_due_at,
        status.last_success_at.map(|time| time + 3_600)
    );
    assert!(status.last_error.is_none());
}

#[test]
fn manual_backup_requires_a_folder_but_can_run_when_configured() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let mut service = BackupService::open(&ledger).expect("open service");
    assert!(service.run_backup(false, 200).is_err());
    service
        .save_settings(
            &BackupSettings {
                directory: Some(directory.path().to_path_buf()),
                ..BackupSettings::default()
            },
            200,
        )
        .expect("save manual folder");
    assert!(
        service
            .run_backup(false, 200)
            .expect("manual backup")
            .is_some()
    );
}

#[test]
fn failed_automatic_backup_records_error_and_retries_after_five_minutes() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let backup_directory = directory.path().join("backups");
    std::fs::create_dir(&backup_directory).expect("create backup folder");
    let mut service = BackupService::open(&ledger).expect("open service");
    service
        .save_settings(
            &BackupSettings {
                directory: Some(backup_directory.clone()),
                mode: BackupMode::Automatic,
                frequency: BackupFrequency::Daily,
            },
            300,
        )
        .expect("save settings");
    std::fs::remove_dir(&backup_directory).expect("remove destination folder");

    assert!(service.run_backup(true, 300).is_err());
    let status = service.status().expect("read failed status");
    assert!(status.last_error.is_some());
    assert_eq!(status.last_success_at, None);
    assert!(
        status
            .next_due_at
            .is_some_and(|time| (600..610).contains(&time))
    );
}

#[test]
fn automatic_mode_requires_an_existing_writable_absolute_directory() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let mut service = BackupService::open(&ledger).expect("open service");
    let without_folder = BackupSettings {
        mode: BackupMode::Automatic,
        ..BackupSettings::default()
    };
    assert!(service.save_settings(&without_folder, 0).is_err());
    let relative = BackupSettings {
        directory: Some(PathBuf::from("relative")),
        mode: BackupMode::Manual,
        frequency: BackupFrequency::Daily,
    };
    assert!(service.save_settings(&relative, 0).is_err());
}

#[test]
fn active_lease_blocks_a_second_run_and_settings_changes_then_expiry_allows_recovery() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let mut first = BackupService::open(&ledger).expect("open first service");
    let settings = BackupSettings {
        directory: Some(directory.path().to_path_buf()),
        mode: BackupMode::Automatic,
        frequency: BackupFrequency::Daily,
    };
    first.save_settings(&settings, 300).expect("save settings");

    let external = Connection::open(&ledger).expect("open lease connection");
    external
        .execute(
            "UPDATE backup_service_state SET lease_token = 'active', lease_until = 400 WHERE id = 1",
            [],
        )
        .expect("claim lease");
    let mut second = BackupService::open(&ledger).expect("open second service");
    assert!(
        second
            .run_backup(true, 300)
            .expect("second run skips")
            .is_none()
    );
    assert!(second.save_settings(&settings, 300).is_err());

    external
        .execute(
            "UPDATE backup_service_state SET lease_until = 299 WHERE id = 1",
            [],
        )
        .expect("expire lease");
    assert!(
        second
            .run_backup(true, 300)
            .expect("expired lease is recovered")
            .is_some()
    );
}

#[test]
fn failed_retry_keeps_the_previous_success_and_backup_path() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let backup_directory = directory.path().join("backups");
    std::fs::create_dir(&backup_directory).expect("create backup folder");
    let mut service = BackupService::open(&ledger).expect("open service");
    let initial = BackupSettings {
        directory: Some(backup_directory.clone()),
        mode: BackupMode::Automatic,
        frequency: BackupFrequency::Daily,
    };
    service.save_settings(&initial, 500).expect("save settings");
    let previous = service
        .run_backup(true, 500)
        .expect("successful backup")
        .expect("backup ran");

    let missing = directory.path().join("removed-backups");
    std::fs::create_dir(&missing).expect("create second folder");
    let changed = BackupSettings {
        directory: Some(missing.clone()),
        ..initial
    };
    service
        .save_settings(&changed, 1000)
        .expect("change folder");
    std::fs::remove_dir(&missing).expect("remove second folder");

    assert!(service.run_backup(true, 1000).is_err());
    let status = service.status().expect("read failed status");
    assert!(status.last_success_at.is_some_and(|time| time >= 500));
    assert_eq!(status.last_backup_path, Some(previous.path));
    assert!(status.last_error.is_some());
    assert!(
        status
            .next_due_at
            .is_some_and(|time| (1300..1310).contains(&time))
    );
}

#[test]
fn backup_includes_committed_wal_changes() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let ledger = existing_ledger(directory.path());
    let mut service = BackupService::open(&ledger).expect("open service");
    service
        .save_settings(
            &BackupSettings {
                directory: Some(directory.path().to_path_buf()),
                ..BackupSettings::default()
            },
            1_800_000_000,
        )
        .expect("save settings");

    let writer = Connection::open(&ledger).expect("open ledger writer");
    writer
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE wal_snapshot_check(value INTEGER NOT NULL);
             INSERT INTO wal_snapshot_check(value) VALUES (42);",
        )
        .expect("write committed WAL record");
    let backup = service
        .run_backup(false, 1_800_000_000)
        .expect("run backup")
        .expect("backup created");

    let snapshot = Connection::open(backup.path).expect("open backup snapshot");
    let value: i64 = snapshot
        .query_row("SELECT value FROM wal_snapshot_check", [], |row| row.get(0))
        .expect("read WAL snapshot record");
    assert_eq!(value, 42);
}
