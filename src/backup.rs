//! Consistent, no-clobber snapshots of the live SQLite ledger.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags};

#[derive(Clone, Debug)]
pub struct BackupSummary {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub elapsed_ms: u128,
}

/// Run on a background worker: the snapshot is streamed page by page and verified
/// before publication. Existing files and live SQLite sidecars are never replaced.
pub fn backup_database(source: &Path, destination: &Path) -> Result<BackupSummary, String> {
    let started = Instant::now();
    let source = source
        .canonicalize()
        .map_err(|e| format!("Cannot locate ledger: {e}"))?;
    let filename = destination.file_name().ok_or("Choose a backup filename")?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = parent
        .canonicalize()
        .map_err(|e| format!("Cannot locate backup folder: {e}"))?
        .join(filename);
    if same_path(&destination, &source)
        || ["-wal", "-shm", "-journal"].iter().any(|suffix| {
            let mut sidecar = source.as_os_str().to_os_string();
            sidecar.push(suffix);
            same_path(&destination, &PathBuf::from(sidecar))
        })
    {
        return Err("Choose a backup path outside the live database and its sidecars".into());
    }
    // symlink_metadata also rejects dangling symlinks; exists() would miss them.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut path = destination.as_os_str().to_os_string();
        path.push(suffix);
        match std::fs::symlink_metadata(PathBuf::from(path)) {
            Ok(_) => return Err("Backup destination or its SQLite sidecars already exist".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Cannot inspect backup destination: {e}")),
        }
    }
    let source_connection = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("Cannot open ledger for backup: {e}"))?;
    source_connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    let temporary =
        tempfile::NamedTempFile::new_in(destination.parent().ok_or("Invalid backup folder")?)
            .map_err(|e| format!("Cannot create temporary backup: {e}"))?;
    {
        let mut target =
            Connection::open(temporary.path()).map_err(|e| format!("Cannot open backup: {e}"))?;
        target
            .progress_handler(
                10_000,
                Some(move || started.elapsed() > Duration::from_secs(120)),
            )
            .map_err(|error| format!("Cannot set backup deadline: {error}"))?;
        {
            let backup = Backup::new(&source_connection, &mut target)
                .map_err(|e| format!("Cannot start backup: {e}"))?;
            loop {
                if started.elapsed() > Duration::from_secs(120) {
                    return Err("Backup timed out; retry when the ledger is less busy".into());
                }
                match backup
                    .step(256)
                    .map_err(|e| format!("Backup failed: {e}"))?
                {
                    StepResult::Done => break,
                    StepResult::More | StepResult::Busy | StepResult::Locked => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    _ => return Err("Unexpected SQLite backup state".into()),
                }
            }
        }
        // Publish a standalone database, including changes currently in the source WAL.
        target
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|e| format!("Cannot finalize backup: {e}"))?;
        let integrity: String = target
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|e| format!("Cannot validate backup: {e}"))?;
        if integrity != "ok" {
            return Err("Backup integrity validation failed".into());
        }
        target
            .close()
            .map_err(|(_, e)| format!("Cannot close backup: {e}"))?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| format!("Cannot flush backup: {e}"))?;
    let size_bytes = temporary
        .as_file()
        .metadata()
        .map_err(|e| e.to_string())?
        .len();
    // Atomic no-clobber publication protects against another process creating the
    // destination after the initial inspection. Tempfile cleans failed snapshots.
    temporary
        .persist_noclobber(&destination)
        .map_err(|e| format!("Cannot publish backup: {e}"))?;
    Ok(BackupSummary {
        path: destination,
        size_bytes,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    // Windows ledger paths must also protect sidecars under case-insensitive names.
    #[cfg(windows)]
    {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_committed_wal_records_and_preserves_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("ledger.sqlite");
        let connection = Connection::open(&source).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE ledger(amount INTEGER); INSERT INTO ledger VALUES (123);").unwrap();
        let destination = directory.path().join("backup.sqlite");
        let summary = backup_database(&source, &destination).unwrap();
        assert!(summary.size_bytes > 0);
        let backup = Connection::open(&destination).unwrap();
        assert_eq!(
            backup
                .query_row("SELECT amount FROM ledger", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            123
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn rejects_existing_destination_source_and_sidecars() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("ledger.sqlite");
        Connection::open(&source)
            .unwrap()
            .execute_batch("CREATE TABLE ledger(amount INTEGER)")
            .unwrap();
        assert!(backup_database(&source, &source).is_err());
        assert!(backup_database(&source, &directory.path().join("ledger.sqlite-wal")).is_err());
        let destination = directory.path().join("existing.sqlite");
        std::fs::write(&destination, b"keep me").unwrap();
        assert!(backup_database(&source, &destination).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"keep me");
    }

    #[test]
    fn corrupt_source_leaves_no_published_or_temporary_backup() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("corrupt.sqlite");
        std::fs::write(&source, b"not a database").unwrap();
        assert!(backup_database(&source, &directory.path().join("backup.sqlite")).is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
