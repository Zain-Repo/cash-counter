//! Persistent backup preferences and due-time scheduling.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::{
    backup::{BackupSummary, backup_database},
    database::Database,
};

const RETRY_DELAY_SECONDS: i64 = 5 * 60;
const LEASE_SECONDS: i64 = 5 * 60;

/// How backups are initiated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupMode {
    /// Create backups only when requested by the user.
    Manual,
    /// Create backups when the configured interval is due.
    Automatic,
}

impl BackupMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "manual" => Ok(Self::Manual),
            "automatic" => Ok(Self::Automatic),
            _ => Err("Stored backup mode is invalid".into()),
        }
    }
}

/// Interval between successful automatic backups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupFrequency {
    /// Back up every hour.
    Hourly,
    /// Back up every six hours.
    EverySixHours,
    /// Back up every day.
    Daily,
    /// Back up every week.
    Weekly,
}

impl BackupFrequency {
    /// Human-readable interval label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hourly => "Hourly",
            Self::EverySixHours => "Every 6 hours",
            Self::Daily => "Daily",
            Self::Weekly => "Weekly",
        }
    }

    /// Interval length in seconds.
    pub fn seconds(self) -> i64 {
        match self {
            Self::Hourly => 3_600,
            Self::EverySixHours => 21_600,
            Self::Daily => 86_400,
            Self::Weekly => 604_800,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::EverySixHours => "six_hours",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "hourly" => Ok(Self::Hourly),
            "six_hours" => Ok(Self::EverySixHours),
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            _ => Err("Stored backup frequency is invalid".into()),
        }
    }
}

/// User-selected backup destination and schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupSettings {
    /// Existing folder for generated backup files.
    pub directory: Option<PathBuf>,
    /// Manual or scheduled backups.
    pub mode: BackupMode,
    /// Interval used when `mode` is automatic.
    pub frequency: BackupFrequency,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            directory: None,
            mode: BackupMode::Manual,
            frequency: BackupFrequency::Daily,
        }
    }
}

/// Backup preferences and most recent scheduler outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupStatus {
    /// Current user-selected preferences.
    pub settings: BackupSettings,
    /// UTC Unix timestamp for the last completed backup.
    pub last_success_at: Option<i64>,
    /// Path of the last completed backup.
    pub last_backup_path: Option<PathBuf>,
    /// UTC Unix timestamp when an automatic backup becomes due.
    pub next_due_at: Option<i64>,
    /// Most recent backup failure, if any.
    pub last_error: Option<String>,
}

/// Reads and updates backup policy in the existing cash-counter database.
pub struct BackupService {
    database: Database,
    source: PathBuf,
}

impl BackupService {
    /// Opens the existing ledger and creates the additive backup metadata table.
    pub fn open(source: &Path) -> Result<Self, String> {
        if !source.is_file() {
            return Err("The ledger database does not exist".into());
        }
        let source = source
            .canonicalize()
            .map_err(|e| format!("Cannot locate ledger: {e}"))?;
        let database = Database::open(&source).map_err(|e| format!("Cannot open ledger: {e}"))?;
        database
            .report_connection()
            .busy_timeout(Duration::from_secs(5))
            .map_err(|e| format!("Cannot configure backup storage: {e}"))?;
        database.report_connection().execute_batch(
            "CREATE TABLE IF NOT EXISTS backup_service_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                directory TEXT,
                mode TEXT NOT NULL CHECK (mode IN ('manual', 'automatic')),
                frequency TEXT NOT NULL CHECK (frequency IN ('hourly', 'six_hours', 'daily', 'weekly')),
                last_success_at INTEGER,
                last_backup_path TEXT,
                next_due_at INTEGER,
                last_error TEXT,
                lease_token TEXT,
                lease_until INTEGER
            );
            INSERT OR IGNORE INTO backup_service_state(id, mode, frequency) VALUES (1, 'manual', 'daily');"
        ).map_err(|e| format!("Cannot initialize backup preferences: {e}"))?;
        Ok(Self { database, source })
    }

    /// Reads saved settings and recent scheduler state.
    pub fn status(&self) -> Result<BackupStatus, String> {
        read_status(self.database.report_connection())
    }

    /// Validates and persists backup preferences.
    pub fn save_settings(
        &mut self,
        settings: &BackupSettings,
        now: i64,
    ) -> Result<BackupStatus, String> {
        let directory = match &settings.directory {
            Some(path) => Some(validate_directory(path)?),
            None if settings.mode == BackupMode::Automatic => {
                return Err("Choose a backup folder before enabling automatic backups".into());
            }
            None => None,
        };
        let connection = self.database.report_connection();
        with_immediate_transaction(connection, "Cannot save backup preferences", |connection| {
            let busy: bool = connection
                .query_row(
                    "SELECT COALESCE(lease_until > ?1, 0) FROM backup_service_state WHERE id = 1",
                    [now],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Cannot inspect backup run: {e}"))?;
            if busy {
                return Err("A backup is currently running".into());
            }
            let old = connection.query_row("SELECT directory, mode, frequency, next_due_at FROM backup_service_state WHERE id = 1", [], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?))).map_err(|e| format!("Cannot read backup preferences: {e}"))?;
            let changed = old.0 != directory.as_ref().map(|p| p.to_string_lossy().into_owned())
                || old.1 != settings.mode.as_str()
                || old.2 != settings.frequency.as_str();
            let next_due = match settings.mode {
                BackupMode::Manual => None,
                BackupMode::Automatic if changed => Some(now),
                BackupMode::Automatic => old.3,
            };
            connection.execute("UPDATE backup_service_state SET directory = ?1, mode = ?2, frequency = ?3, next_due_at = ?4, last_error = CASE WHEN ?5 THEN NULL ELSE last_error END WHERE id = 1", params![directory.map(|p| p.to_string_lossy().into_owned()), settings.mode.as_str(), settings.frequency.as_str(), next_due, changed]).map_err(|e| format!("Cannot save backup preferences: {e}"))?;
            Ok(())
        })?;
        self.status()
    }

    /// Runs a requested backup, or an automatic backup when the schedule is due.
    /// Returns `Ok(None)` when an automatic run is not due or another process owns the lease.
    pub fn run_backup(
        &mut self,
        automatic: bool,
        now: i64,
    ) -> Result<Option<BackupSummary>, String> {
        let timestamp: DateTime<Utc> =
            DateTime::from_timestamp(now, 0).ok_or("Invalid backup timestamp")?;
        let token = uuid::Uuid::new_v4().to_string();
        let connection = self.database.report_connection();
        let claimed = with_immediate_transaction(
            connection,
            "Cannot claim backup run",
            |connection| {
                let status = read_status(connection)?;
                if automatic
                    && (status.settings.mode != BackupMode::Automatic
                        || status.next_due_at.is_none_or(|due| now < due))
                {
                    return Ok(None);
                }
                let directory = status
                    .settings
                    .directory
                    .ok_or("Choose a backup folder first")?;
                let claimed = connection.execute("UPDATE backup_service_state SET lease_token = ?1, lease_until = ?2 WHERE id = 1 AND (lease_until IS NULL OR lease_until <= ?3)", params![token, now.saturating_add(LEASE_SECONDS), now]).map_err(|e| format!("Cannot claim backup run: {e}"))?;
                if claimed == 0 {
                    return Ok(None);
                }
                Ok(Some((
                    directory,
                    status.settings.mode,
                    status.settings.frequency,
                )))
            },
        )?;
        let Some((directory, mode, frequency)) = claimed else {
            return Ok(None);
        };

        let started = Instant::now();
        let destination = directory.join(format!(
            "cash-counter-{}-{}.sqlite",
            timestamp.format("%Y%m%dT%H%M%SZ"),
            uuid::Uuid::new_v4()
        ));
        match backup_database(&self.source, &destination) {
            Ok(summary) => {
                let completion =
                    now.saturating_add(started.elapsed().as_secs().min(i64::MAX as u64) as i64);
                self.finish_run(
                    &token,
                    Some((&summary.path, completion)),
                    None,
                    (mode == BackupMode::Automatic)
                        .then_some(completion.saturating_add(frequency.seconds())),
                )?;
                Ok(Some(summary))
            }
            Err(error) => {
                self.finish_run(
                    &token,
                    None,
                    Some(&error),
                    (mode == BackupMode::Automatic).then_some(
                        now.saturating_add(started.elapsed().as_secs().min(i64::MAX as u64) as i64)
                            .saturating_add(RETRY_DELAY_SECONDS),
                    ),
                )?;
                Err(error)
            }
        }
    }

    fn finish_run(
        &self,
        token: &str,
        success: Option<(&Path, i64)>,
        error: Option<&str>,
        next_due: Option<i64>,
    ) -> Result<(), String> {
        let (last_success, path, last_error) = match (success, error) {
            (Some((path, at)), _) => (Some(at), Some(path.to_string_lossy().into_owned()), None),
            (_, Some(error)) => (None, None, Some(error)),
            _ => (None, None, None),
        };
        let changed = self.database.report_connection().execute(
            "UPDATE backup_service_state SET last_success_at = COALESCE(?1, last_success_at), last_backup_path = COALESCE(?2, last_backup_path), last_error = ?3, next_due_at = COALESCE(?4, next_due_at), lease_token = NULL, lease_until = NULL WHERE id = 1 AND lease_token = ?5",
            params![last_success, path, last_error, next_due, token]
        ).map_err(|e| format!("Cannot record backup result: {e}"))?;
        if changed != 1 {
            return Err("Backup run lost its lease before the result could be recorded".into());
        }
        Ok(())
    }
}

fn read_status(connection: &Connection) -> Result<BackupStatus, String> {
    connection
        .query_row(
            "SELECT directory, mode, frequency, last_success_at, last_backup_path, next_due_at, last_error FROM backup_service_state WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .map_err(|e| format!("Cannot read backup preferences: {e}"))
        .and_then(
            |(directory, mode, frequency, last_success_at, last_backup_path, next_due_at, last_error)| {
                Ok(BackupStatus {
                    settings: BackupSettings {
                        directory: directory.map(PathBuf::from),
                        mode: BackupMode::parse(&mode)?,
                        frequency: BackupFrequency::parse(&frequency)?,
                    },
                    last_success_at,
                    last_backup_path: last_backup_path.map(PathBuf::from),
                    next_due_at,
                    last_error,
                })
            },
        )
}

fn with_immediate_transaction<T>(
    connection: &Connection,
    context: &str,
    operation: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| format!("{context}: {e}"))?;
    let value = operation(&transaction)?;
    transaction
        .commit()
        .map_err(|e| format!("{context}: {e}"))?;
    Ok(value)
}

fn validate_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute backup folder path".into());
    }
    let directory = path
        .canonicalize()
        .map_err(|e| format!("Cannot locate backup folder: {e}"))?;
    if !directory.is_dir() {
        return Err("Backup folder must be a directory".into());
    }
    let probe = tempfile::Builder::new()
        .prefix(".cash-counter-backup-check-")
        .tempfile_in(&directory)
        .map_err(|e| format!("Backup folder is not writable: {e}"))?;
    probe
        .close()
        .map_err(|e| format!("Cannot validate backup folder: {e}"))?;
    Ok(directory)
}
