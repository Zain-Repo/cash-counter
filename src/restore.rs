//! Safe restoration of validated cash-counter ledger backups.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::password_hash::PasswordHash;
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
    params,
};

const RESTORE_TIMEOUT: Duration = Duration::from_secs(120);
const CURRENT_SCHEMA: i64 = 3;

/// A completed restore and the no-clobber snapshot made before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreSummary {
    pub safety_backup: PathBuf,
    pub restored_from: PathBuf,
}

/// Read-only confirmation details for a selected backup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePreview {
    pub backup: PathBuf,
    pub size_bytes: u64,
    pub modified_unix_nanos: u128,
    pub sha256: String,
}

/// Resolves and fully validates a backup without changing the live ledger.
pub fn preview_restore(live: &Path, selection: &Path) -> Result<RestorePreview, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let backup = resolve_backup(selection, live)?;
    let metadata = std::fs::metadata(&backup).map_err(|e| format!("Cannot inspect backup: {e}"))?;
    if metadata.len() > 2 * 1024 * 1024 * 1024 {
        return Err("Selected backup exceeds the 2 GiB restore limit".into());
    }
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let connection = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("Cannot open selected backup: {e}"))?;
    validate_database(&connection, Instant::now())?;
    drop(connection);
    let mut file =
        std::fs::File::open(&backup).map_err(|e| format!("Cannot read selected backup: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("Cannot read selected backup: {e}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let sha256 = format!("{:x}", hash.finalize());
    Ok(RestorePreview {
        backup,
        size_bytes: metadata.len(),
        modified_unix_nanos,
        sha256,
    })
}

#[derive(Clone)]
struct BackupState {
    directory: Option<String>,
    mode: String,
    frequency: String,
    last_success_at: Option<i64>,
    last_backup_path: Option<String>,
}

/// Resolves a selected backup file or a folder containing generated backups.
/// A folder selection chooses the newest generated ledger backup and skips recovery snapshots.
pub fn resolve_backup(selection: &Path, live: &Path) -> Result<PathBuf, String> {
    if selection.is_file() {
        let selected = selection
            .canonicalize()
            .map_err(|e| format!("Cannot locate selected backup: {e}"))?;
        let live = live
            .canonicalize()
            .map_err(|e| format!("Cannot locate live ledger: {e}"))?;
        reject_same_or_sidecar(&live, &selected)?;
        return Ok(selected);
    }
    if !selection.is_dir() {
        return Err("Choose an existing backup file or folder".into());
    }
    let mut candidates = Vec::new();
    for entry in
        std::fs::read_dir(selection).map_err(|e| format!("Cannot read backup folder: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Cannot read backup folder entry: {e}"))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("cash-counter-")
            && !name.starts_with("cash-counter-pre-restore-")
            && name.ends_with(".sqlite")
        {
            let metadata = entry
                .metadata()
                .map_err(|e| format!("Cannot inspect backup candidate: {e}"))?;
            if metadata.is_file() {
                candidates.push((
                    metadata
                        .modified()
                        .map_err(|e| format!("Cannot inspect backup timestamp: {e}"))?,
                    path,
                ));
            }
        }
    }
    let selected = candidates
        .into_iter()
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, path)| {
            path.canonicalize()
                .map_err(|e| format!("Cannot locate backup: {e}"))
        })
        .transpose()?
        .ok_or_else(|| "No generated cash-counter backups were found in that folder".to_string())?;
    let live = live
        .canonicalize()
        .map_err(|e| format!("Cannot locate live ledger: {e}"))?;
    reject_same_or_sidecar(&live, &selected)?;
    Ok(selected)
}

/// Restores a verified cash-counter backup while retaining the live backup policy.
pub fn restore_database(live: &Path, backup: &Path) -> Result<RestoreSummary, String> {
    let started = Instant::now();
    let live = live
        .canonicalize()
        .map_err(|e| format!("Cannot locate live ledger: {e}"))?;
    let backup = backup
        .canonicalize()
        .map_err(|e| format!("Cannot locate backup: {e}"))?;
    reject_same_or_sidecar(&live, &backup)?;

    let staged = tempfile::NamedTempFile::new_in(live.parent().ok_or("Invalid live ledger path")?)
        .map_err(|e| format!("Cannot stage restore: {e}"))?;
    {
        let source = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Cannot open selected backup: {e}"))?;
        validate_database(&source, started)?;
        let mut target =
            Connection::open(staged.path()).map_err(|e| format!("Cannot stage restore: {e}"))?;
        copy_database(&source, &mut target, started)?;
        target
            .close()
            .map_err(|(_, e)| format!("Cannot finalize staged backup: {e}"))?;
    }
    // Migrations run only against the private staged copy, before acquiring or changing live data.
    drop(
        crate::database::Database::open(staged.path())
            .map_err(|e| format!("Backup migration failed: {e}"))?,
    );
    {
        let staged_company = crate::company::CompanyService::open(staged.path())
            .map_err(|e| format!("Backup company data is invalid: {e}"))?;
        if staged_company
            .profile()
            .map_err(|e| format!("Backup company profile is invalid: {e}"))?
            .is_none()
        {
            return Err("Backup does not contain a configured Cash Ledger company".into());
        }
    }
    {
        let staged_db =
            Connection::open_with_flags(staged.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| format!("Cannot validate staged backup: {e}"))?;
        validate_database(&staged_db, started)?;
    }

    let safety_path = unique_safety_path(&live)?;
    let mut live_connection =
        Connection::open(&live).map_err(|e| format!("Cannot open live ledger: {e}"))?;
    live_connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    live_connection
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .map_err(|e| format!("Cannot lock live ledger: {e}"))?;
    let settings = {
        let tx = live_connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive)
            .map_err(|e| {
                format!("Cannot lock live ledger; close other ledger windows and retry: {e}")
            })?;
        let schema: i64 = tx
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if schema > CURRENT_SCHEMA {
            return Err("The live ledger uses a newer schema and cannot be restored safely".into());
        }
        read_backup_settings(&tx)?
    };
    {
        let staged_write = Connection::open(staged.path())
            .map_err(|e| format!("Cannot prepare staged backup: {e}"))?;
        restore_settings(&staged_write, settings.clone())?;
        validate_database(&staged_write, started)?;
    }
    // Exclusive locking mode keeps the acquired lock on this connection after transaction rollback.
    let safety_temp =
        tempfile::NamedTempFile::new_in(live.parent().ok_or("Invalid live ledger path")?)
            .map_err(|e| format!("Cannot create safety snapshot: {e}"))?;
    {
        let mut target = Connection::open(safety_temp.path())
            .map_err(|e| format!("Cannot create safety snapshot: {e}"))?;
        copy_database(&live_connection, &mut target, started)?;
        target
            .close()
            .map_err(|(_, e)| format!("Cannot finalize safety snapshot: {e}"))?;
    }
    {
        let safety_check = Connection::open(safety_temp.path())
            .map_err(|e| format!("Cannot validate safety snapshot: {e}"))?;
        safety_check
            .progress_handler(1_000, Some(move || started.elapsed() > RESTORE_TIMEOUT))
            .map_err(|e| format!("Cannot set safety snapshot deadline: {e}"))?;
        let integrity: String = safety_check
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| format!("Cannot validate safety snapshot: {e}"))?;
        if integrity != "ok" {
            return Err("Safety snapshot failed SQLite integrity validation".into());
        }
        safety_check
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|e| format!("Cannot finalize safety snapshot: {e}"))?;
    }
    safety_temp
        .as_file()
        .sync_all()
        .map_err(|e| format!("Cannot flush safety snapshot: {e}"))?;
    safety_temp
        .persist_noclobber(&safety_path)
        .map_err(|e| format!("Cannot publish safety snapshot: {e}"))?;

    {
        let source = Connection::open_with_flags(staged.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("Cannot open staged backup: {e}"))?;
        copy_database(&source, &mut live_connection, started)?;
    }
    drop(live_connection);
    Ok(RestoreSummary {
        safety_backup: safety_path,
        restored_from: backup,
    })
}

fn validate_database(connection: &Connection, started: Instant) -> Result<(), String> {
    connection
        .progress_handler(1_000, Some(move || started.elapsed() > RESTORE_TIMEOUT))
        .map_err(|e| format!("Cannot set restore deadline: {e}"))?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| format!("Selected file is not a valid SQLite ledger: {e}"))?;
    if !(1..=CURRENT_SCHEMA).contains(&version) {
        return Err("Backup schema version is unsupported".into());
    }
    for table in [
        "registers",
        "register_sessions",
        "cash_in",
        "cash_out",
        "company_profile",
        "company_users",
        "company_login_guard",
    ] {
        let found: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |r| r.get(0),
            )
            .map_err(|e| format!("Cannot inspect selected backup: {e}"))?;
        if !found {
            return Err("Selected file is not a configured Cash Ledger database".into());
        }
    }
    for query in [
        "SELECT id,name,currency,created_at FROM registers LIMIT 0",
        "SELECT id,register_id,business_date,opening,cash_in_total,cash_out_total,expected,counted,difference,opened_at,closed_at FROM register_sessions LIMIT 0",
        "SELECT id,session_id,amount,reason,request_key,created_at FROM cash_in LIMIT 0",
        "SELECT id,session_id,amount,reason,request_key,created_at FROM cash_out LIMIT 0",
        "SELECT id,failures,locked_until FROM company_login_guard LIMIT 0",
    ] {
        connection
            .prepare(query)
            .map_err(|_| "Selected file has an unsupported Cash Ledger schema".to_string())?;
    }
    let profile: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM company_profile WHERE id=1)",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("Backup company profile is invalid: {e}"))?;
    let credentials: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM company_users)", [], |r| {
            r.get(0)
        })
        .map_err(|e| format!("Backup company credentials are invalid: {e}"))?;
    if !profile || !credentials {
        return Err("Backup does not contain a complete company profile and credentials".into());
    }
    let profile_values: (String, String, String, i64, String, String) = connection
        .query_row(
            "SELECT name,address,rules,variance_threshold,printer_name,paper_size FROM company_profile WHERE id=1",
            [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        )
        .map_err(|_| "Selected file does not contain a valid Cash Ledger company profile".to_string())?;
    let (name, address, rules, threshold, printer, paper_size) = profile_values;
    if name.trim().is_empty()
        || name.len() > 160
        || address.len() > 1000
        || rules.len() > 4000
        || printer.len() > 256
        || !(0..=100_000_000).contains(&threshold)
        || !matches!(paper_size.as_str(), "A4" | "Letter")
        || name.chars().chain(printer.chars()).any(char::is_control)
        || address
            .chars()
            .chain(rules.chars())
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("Backup contains an invalid Cash Ledger company profile".into());
    }
    let mut users = connection
        .prepare("SELECT username,password_hash FROM company_users")
        .map_err(|_| "Selected file does not contain valid Cash Ledger credentials".to_string())?;
    let hashes = users
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut valid_credentials = false;
    for hash in hashes {
        let (username, hash) = hash.map_err(|e| e.to_string())?;
        if username.trim().is_empty()
            || username.len() > 64
            || !username
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'@'))
        {
            return Err("Backup contains invalid Cash Ledger credentials".into());
        }
        let parsed = PasswordHash::new(&hash)
            .map_err(|_| "Backup contains invalid company credentials".to_string())?;
        if parsed.algorithm.as_str() != "argon2id" || parsed.salt.is_none() || parsed.hash.is_none()
        {
            return Err("Backup contains invalid company credentials".into());
        }
        let memory = parsed.params.get_decimal("m");
        let iterations = parsed.params.get_decimal("t");
        let lanes = parsed.params.get_decimal("p");
        if !matches!(memory, Some(8..=262_144))
            || !matches!(iterations, Some(1..=10))
            || !matches!(lanes, Some(1..=16))
        {
            return Err("Backup contains unsupported company credential parameters".into());
        }
        valid_credentials = true;
    }
    if !valid_credentials {
        return Err("Backup does not contain valid company credentials".into());
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| format!("Cannot check backup integrity: {e}"))?;
    if integrity != "ok" {
        return Err("Backup failed SQLite integrity validation".into());
    }
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| e.to_string())?;
    if statement.exists([]).map_err(|e| e.to_string())? {
        return Err("Backup failed foreign-key validation".into());
    }
    Ok(())
}

fn copy_database(
    source: &Connection,
    target: &mut Connection,
    started: Instant,
) -> Result<(), String> {
    target
        .progress_handler(10_000, Some(move || started.elapsed() > RESTORE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let backup =
        Backup::new(source, target).map_err(|e| format!("SQLite restore copy failed: {e}"))?;
    loop {
        if started.elapsed() > RESTORE_TIMEOUT {
            return Err("Restore timed out; retry when the ledger is less busy".into());
        }
        match backup
            .step(256)
            .map_err(|e| format!("SQLite restore copy failed: {e}"))?
        {
            StepResult::Done => return Ok(()),
            StepResult::More | StepResult::Busy | StepResult::Locked => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => return Err("Unexpected SQLite restore state".into()),
        }
    }
}

fn read_backup_settings(tx: &rusqlite::Transaction<'_>) -> Result<BackupState, String> {
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='backup_service_state')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !exists {
        return Ok(BackupState {
            directory: None,
            mode: "manual".into(),
            frequency: "daily".into(),
            last_success_at: None,
            last_backup_path: None,
        });
    }
    let active_lease: bool = tx
        .query_row(
            "SELECT COALESCE(lease_until > ?1, 0) FROM backup_service_state WHERE id=1",
            [now_seconds()],
            |r| r.get(0),
        )
        .map_err(|e| format!("Cannot inspect backup activity: {e}"))?;
    if active_lease {
        return Err("A backup is currently running; retry after it finishes".into());
    }
    tx.query_row("SELECT directory,mode,frequency,last_success_at,last_backup_path FROM backup_service_state WHERE id=1", [], |r| Ok(BackupState { directory: r.get(0)?, mode: r.get(1)?, frequency: r.get(2)?, last_success_at: r.get(3)?, last_backup_path: r.get(4)? })).map_err(|e| format!("Cannot retain backup settings: {e}"))
}

fn restore_settings(connection: &Connection, settings: BackupState) -> Result<(), String> {
    let next_due = if settings.mode == "automatic" {
        let seconds = match settings.frequency.as_str() {
            "hourly" => 3600,
            "six_hours" => 21600,
            "weekly" => 604800,
            _ => 86400,
        };
        Some(now_seconds().saturating_add(seconds))
    } else {
        None
    };
    connection.execute_batch("CREATE TABLE IF NOT EXISTS backup_service_state (id INTEGER PRIMARY KEY CHECK (id=1), directory TEXT, mode TEXT NOT NULL CHECK(mode IN ('manual','automatic')), frequency TEXT NOT NULL CHECK(frequency IN ('hourly','six_hours','daily','weekly')), last_success_at INTEGER, last_backup_path TEXT, next_due_at INTEGER, last_error TEXT, lease_token TEXT, lease_until INTEGER);")
        .map_err(|e| format!("Cannot restore backup settings: {e}"))?;
    connection.execute("INSERT INTO backup_service_state(id,directory,mode,frequency,last_success_at,last_backup_path,next_due_at,last_error,lease_token,lease_until) VALUES(1,?1,?2,?3,?4,?5,?6,NULL,NULL,NULL) ON CONFLICT(id) DO UPDATE SET directory=excluded.directory,mode=excluded.mode,frequency=excluded.frequency,last_success_at=excluded.last_success_at,last_backup_path=excluded.last_backup_path,next_due_at=excluded.next_due_at,last_error=NULL,lease_token=NULL,lease_until=NULL", params![settings.directory,settings.mode,settings.frequency,settings.last_success_at,settings.last_backup_path,next_due]).map_err(|e| format!("Cannot restore backup settings: {e}"))?;
    Ok(())
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}

fn unique_safety_path(live: &Path) -> Result<PathBuf, String> {
    let parent = live.parent().ok_or("Invalid live ledger path")?;
    for _ in 0..8 {
        let path = parent.join(format!(
            "cash-counter-pre-restore-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("Cannot choose a unique safety snapshot path".into())
}

fn reject_same_or_sidecar(live: &Path, backup: &Path) -> Result<(), String> {
    if same_path(live, backup) {
        return Err("Choose a backup separate from the live ledger".into());
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = live.as_os_str().to_os_string();
        sidecar.push(suffix);
        if same_path(Path::new(&sidecar), backup) {
            return Err("A SQLite sidecar cannot be restored as a backup".into());
        }
    }
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}
