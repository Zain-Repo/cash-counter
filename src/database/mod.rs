//! SQLite storage with append-only movements and incrementally maintained balances.
//!
//! Amounts are signed 64-bit integer minor units. One connection belongs to one
//! caller; UI integration should move this synchronous service to a worker thread.

#[cfg(test)]
mod tests;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{path::Path, time::Duration};

/// Errors returned by cash-register storage operations.
#[derive(Debug)]
pub enum Error {
    /// SQLite I/O, locking or integrity failure; retain for operator diagnostics.
    Sqlite(rusqlite::Error),
    /// Invalid pagination bounds or conflicting request-key reuse.
    InvalidInput(&'static str),
    /// The file was created by a newer or unsupported schema.
    UnsupportedSchema(i64),
    /// The requested session is missing or already closed.
    SessionNotOpen,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(_) => formatter.write_str("Database operation failed"),
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "Unsupported database schema version: {version}")
            }
            Self::SessionNotOpen => {
                formatter.write_str("Session does not exist or is already closed")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Result type for the database API.
pub type Result<T> = std::result::Result<T, Error>;

/// Selects a cash movement table. Entry IDs and retry keys are scoped by direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Cash received.
    In,
    /// Cash paid out.
    Out,
}

impl Direction {
    fn insert_sql(self) -> &'static str {
        match self {
            Self::In => {
                "INSERT INTO cash_in(session_id, amount, reason, request_key) VALUES (?1, ?2, ?3, ?4)"
            }
            Self::Out => {
                "INSERT INTO cash_out(session_id, amount, reason, request_key) VALUES (?1, ?2, ?3, ?4)"
            }
        }
    }

    fn retry_sql(self) -> &'static str {
        match self {
            Self::In => "SELECT id, session_id, amount, reason FROM cash_in WHERE request_key = ?1",
            Self::Out => {
                "SELECT id, session_id, amount, reason FROM cash_out WHERE request_key = ?1"
            }
        }
    }

    fn page_sql(self) -> &'static str {
        match self {
            Self::In => {
                "SELECT id, amount, reason, request_key, created_at FROM cash_in WHERE session_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3"
            }
            Self::Out => {
                "SELECT id, amount, reason, request_key, created_at FROM cash_out WHERE session_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3"
            }
        }
    }
}

/// A new movement; reuse the same request key when retrying the same operation.
#[derive(Debug)]
pub struct NewEntry<'a> {
    pub direction: Direction,
    pub amount: i64,
    pub reason: &'a str,
    pub request_key: &'a str,
}

/// An immutable persisted movement.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub amount: i64,
    pub reason: String,
    pub request_key: String,
    /// UTC timestamp assigned by SQLite.
    pub created_at: String,
}

/// Exact totals for a session; closing fields are absent until counted.
#[derive(Debug, PartialEq, Eq)]
pub struct Balance {
    pub opening: i64,
    pub cash_in: i64,
    pub cash_out: i64,
    pub expected: i64,
    pub counted: Option<i64>,
    pub difference: Option<i64>,
}

/// Register metadata used by the workspace selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub id: i64,
    pub name: String,
    pub currency: String,
}

/// A daily session's identity and closing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: i64,
    pub register_id: i64,
    pub business_date: String,
    pub closed: bool,
}

/// Owns a SQLite connection. The connection is private to preserve ledger invariants.
///
/// ```
/// use cash_counter::database::{Database, Direction, NewEntry};
/// let mut db = Database::in_memory()?;
/// let register = db.create_register("Front desk", "CAD")?;
/// let session = db.open_session(register, "2026-10-08", 10_000)?;
/// db.record_entries(session, &[NewEntry {
///     direction: Direction::In,
///     amount: 2_500,
///     reason: "Cash sale",
///     request_key: "sale-001",
/// }])?;
/// assert_eq!(db.balance(session)?.expected, 12_500);
/// db.close_session(session, 12_500)?;
/// # Ok::<(), cash_counter::database::Error>(())
/// ```
pub struct Database {
    connection: Connection,
}

impl Database {
    /// Allows the reporting module to read a consistent ledger snapshot.
    pub(crate) fn report_connection(&self) -> &Connection {
        &self.connection
    }

    /// Finds the register's open session using the partial unique index.
    pub fn active_session(&self, register: i64) -> Result<Option<Session>> {
        self.connection.query_row(
            "SELECT id, register_id, business_date, closed_at IS NOT NULL FROM register_sessions WHERE register_id = ?1 AND closed_at IS NULL",
            [register], |row| Ok(Session { id: row.get(0)?, register_id: row.get(1)?, business_date: row.get(2)?, closed: row.get(3)? }),
        ).optional().map_err(Error::from)
    }
    /// Lists register metadata without loading movement history.
    pub fn registers(&self) -> Result<Vec<Register>> {
        self.connection
            .prepare("SELECT id, name, currency FROM registers ORDER BY id")?
            .query_map([], |row| {
                Ok(Register {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    currency: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }

    /// Reads sessions newest-first with a bounded primary-key cursor.
    pub fn sessions(
        &self,
        register: i64,
        before_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Session>> {
        if !(1..=1000).contains(&limit) || before_id.is_some_and(|id| id <= 0) {
            return Err(Error::InvalidInput("Invalid session page bounds"));
        }
        let sql = if before_id.is_some() {
            "SELECT id, register_id, business_date, closed_at IS NOT NULL FROM register_sessions WHERE register_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
        } else {
            "SELECT id, register_id, business_date, closed_at IS NOT NULL FROM register_sessions WHERE register_id = ?1 ORDER BY id DESC LIMIT ?3"
        };
        self.connection
            .prepare_cached(sql)?
            .query_map(params![register, before_id, limit], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    register_id: row.get(1)?,
                    business_date: row.get(2)?,
                    closed: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }

    /// Looks up a daily session by its primary key.
    pub fn session(&self, id: i64) -> Result<Session> {
        self.connection.query_row(
            "SELECT id, register_id, business_date, closed_at IS NOT NULL FROM register_sessions WHERE id = ?1",
            [id], |row| Ok(Session { id: row.get(0)?, register_id: row.get(1)?, business_date: row.get(2)?, closed: row.get(3)? }),
        ).map_err(Error::from)
    }

    /// Returns a bounded newest-first page of movement history.
    pub fn recent_entries(
        &self,
        session: i64,
        direction: Direction,
        before_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Entry>> {
        if !(1..=1000).contains(&limit) || before_id.is_some_and(|id| id <= 0) {
            return Err(Error::InvalidInput("Invalid history page bounds"));
        }
        let sql = match (direction, before_id.is_some()) {
            (Direction::In, true) => {
                "SELECT id, amount, reason, request_key, created_at FROM cash_in WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
            }
            (Direction::In, false) => {
                "SELECT id, amount, reason, request_key, created_at FROM cash_in WHERE session_id = ?1 ORDER BY id DESC LIMIT ?3"
            }
            (Direction::Out, true) => {
                "SELECT id, amount, reason, request_key, created_at FROM cash_out WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
            }
            (Direction::Out, false) => {
                "SELECT id, amount, reason, request_key, created_at FROM cash_out WHERE session_id = ?1 ORDER BY id DESC LIMIT ?3"
            }
        };
        self.connection
            .prepare_cached(sql)?
            .query_map(params![session, before_id, limit], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    amount: row.get(1)?,
                    reason: row.get(2)?,
                    request_key: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }
    /// Opens or creates a database at an explicit path and applies atomic migrations.
    /// Parent directories must already exist. Rejects newer schema versions.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::initialize(Connection::open(path)?)
    }

    /// Creates an isolated temporary database for tests or previews.
    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        // REPLACE must not bypass the append-only delete guards.
        connection.pragma_update(None, "recursive_triggers", true)?;
        // FULL preserves committed financial records on power loss in WAL mode.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                transaction.execute_batch(include_str!("migrations/001_cash_register.sql"))?;
                transaction.execute_batch(include_str!("migrations/002_session_history.sql"))?;
                transaction.pragma_update(None, "user_version", 2)?;
            }
            1 => {
                transaction.execute_batch(include_str!("migrations/002_session_history.sql"))?;
                transaction.pragma_update(None, "user_version", 2)?;
            }
            2 => {}
            _ => {
                return Err(Error::UnsupportedSchema(version));
            }
        }
        transaction.commit()?;
        Ok(Self { connection })
    }

    /// Creates a register with a three-letter uppercase currency code.
    /// Currency minor-unit conventions are the caller's responsibility.
    pub fn create_register(&mut self, name: &str, currency: &str) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO registers(name, currency) VALUES (?1, ?2)",
            params![name, currency],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Opens one session per register and business date (YYYY-MM-DD).
    /// The caller supplies the local business date and an explicit opening count.
    pub fn open_session(&mut self, register: i64, date: &str, opening: i64) -> Result<i64> {
        self.connection.execute("INSERT INTO register_sessions(register_id, business_date, opening) VALUES (?1, ?2, ?3)", params![register, date, opening])?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Writes an entire batch atomically and returns direction-scoped entry IDs.
    /// An identical retry returns the original ID; conflicting reuse is rejected.
    /// Constraints reject nonpositive amounts, empty text, closed sessions and overflow.
    pub fn record_entries(&mut self, session: i64, entries: &[NewEntry<'_>]) -> Result<Vec<i64>> {
        // Serializing writers before retry lookup prevents concurrent duplicate retries.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut ids = Vec::with_capacity(entries.len());
        for entry in entries {
            let existing: Option<(i64, i64, i64, String)> = transaction
                .prepare_cached(entry.direction.retry_sql())?
                .query_row([entry.request_key], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .optional()?;
            if let Some((id, previous_session, amount, reason)) = existing {
                if previous_session != session || amount != entry.amount || reason != entry.reason {
                    return Err(Error::InvalidInput(
                        "Request key conflicts with an existing entry",
                    ));
                }
                ids.push(id);
            } else {
                transaction
                    .prepare_cached(entry.direction.insert_sql())?
                    .execute(params![
                        session,
                        entry.amount,
                        entry.reason,
                        entry.request_key
                    ])?;
                ids.push(transaction.last_insert_rowid());
            }
        }
        transaction.commit()?;
        Ok(ids)
    }

    /// Reads stored totals by primary key, without scanning movement history.
    pub fn balance(&self, session: i64) -> Result<Balance> {
        self.connection
            .query_row(
                "SELECT opening, cash_in_total, cash_out_total, expected, counted, difference
             FROM register_sessions WHERE id = ?1",
                [session],
                |row| {
                    Ok(Balance {
                        opening: row.get(0)?,
                        cash_in: row.get(1)?,
                        cash_out: row.get(2)?,
                        expected: row.get(3)?,
                        counted: row.get(4)?,
                        difference: row.get(5)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    /// Permanently closes an open session with the physical cash count.
    pub fn close_session_if_unchanged(
        &mut self,
        session: i64,
        counted: i64,
        expected: i64,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE register_sessions SET counted = ?1, closed_at = CURRENT_TIMESTAMP
             WHERE id = ?2 AND closed_at IS NULL AND expected = ?3",
            params![counted, session, expected],
        )?;
        if changed == 0 {
            return Err(Error::InvalidInput(
                "This day's balance or status changed. Refresh and review the count again.",
            ));
        }
        Ok(())
    }

    /// Permanently closes an open session with the physical cash count.
    pub fn close_session(&mut self, session: i64, counted: i64) -> Result<()> {
        let changed = self.connection.execute("UPDATE register_sessions SET counted = ?1, closed_at = CURRENT_TIMESTAMP WHERE id = ?2 AND closed_at IS NULL", params![counted, session])?;
        if changed == 0 {
            return Err(Error::SessionNotOpen);
        }
        Ok(())
    }

    /// Reads at most 1,000 entries using an indexed cursor rather than OFFSET.
    /// Start with after_id = 0; pass the last returned ID for the next page.
    pub fn entries(
        &self,
        session: i64,
        direction: Direction,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<Entry>> {
        if after_id < 0 || !(1..=1000).contains(&limit) {
            return Err(Error::InvalidInput("Invalid history page bounds"));
        }
        self.connection
            .prepare_cached(direction.page_sql())?
            .query_map(params![session, after_id, limit], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    amount: row.get(1)?,
                    reason: row.get(2)?,
                    request_key: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }
}
