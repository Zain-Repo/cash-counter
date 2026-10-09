//! Application operations shared by the GPUI workspace and integration tests.

use crate::database::{Balance, Database, Direction, Entry, NewEntry, Register, Session};
use std::path::Path;

pub mod money;

/// Bounded history size for the register workspace.
pub const PAGE_SIZE: u32 = 25;

/// Selection and cursors for a workspace snapshot.
#[derive(Clone, Debug)]
pub struct Selection {
    pub register: Option<i64>,
    pub session: Option<i64>,
    pub direction: Direction,
    pub before_entry: Option<i64>,
    pub before_session: Option<i64>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            register: None,
            session: None,
            direction: Direction::In,
            before_entry: None,
            before_session: None,
        }
    }
}

/// An explicit user operation; no sample financial records are generated.
#[derive(Clone, Debug)]
pub enum Operation {
    Refresh,
    CreateRegister {
        name: String,
    },
    Open {
        register: i64,
        date: String,
        opening: i64,
    },
    Record {
        session: i64,
        direction: Direction,
        amount: i64,
        reason: String,
        request_key: String,
    },
    Close {
        session: i64,
        counted: i64,
        expected: i64,
    },
}

/// Persisted state displayed by the UI.
#[derive(Debug)]
pub struct Snapshot {
    pub registers: Vec<Register>,
    pub register: Option<Register>,
    pub sessions: Vec<Session>,
    pub session: Option<Session>,
    pub balance: Option<Balance>,
    pub entries: Vec<Entry>,
    pub more_entries: bool,
    pub more_sessions: bool,
    pub active_session: Option<Session>,
}

/// Separates a successful write from a subsequent failed refresh.
pub struct Reply {
    pub applied: bool,
    /// Failed record writes may be retried only with their original payload.
    pub retryable: bool,
    pub snapshot: Result<Snapshot, String>,
}

/// Synchronous storage service. Run on a background executor, never during render.
pub struct RegisterService {
    db: Database,
}

impl RegisterService {
    /// Opens the application database without creating registers or sessions.
    pub fn open(path: impl AsRef<Path>) -> crate::database::Result<Self> {
        Ok(Self {
            db: Database::open(path)?,
        })
    }

    /// Applies an operation and reloads a bounded workspace snapshot.
    pub fn execute(&mut self, operation: &Operation, selection: &Selection) -> Reply {
        let mut selection = selection.clone();
        let result = self.apply(operation, &mut selection);
        match result {
            Ok(()) => Reply {
                applied: true,
                retryable: false,
                snapshot: self.load(&selection).map_err(|error| error.to_string()),
            },
            Err(error) => {
                let retryable = matches!(operation, Operation::Record { .. })
                    && match &error {
                        crate::database::Error::Sqlite(rusqlite::Error::SqliteFailure(
                            details,
                            _,
                        )) => details.code != rusqlite::ErrorCode::ConstraintViolation,
                        _ => false,
                    };
                Reply {
                    applied: false,
                    retryable,
                    snapshot: Err(friendly_error(&error)),
                }
            }
        }
    }

    fn apply(
        &mut self,
        operation: &Operation,
        selection: &mut Selection,
    ) -> crate::database::Result<()> {
        match operation {
            Operation::Refresh => {}
            Operation::CreateRegister { name } => {
                selection.register = Some(self.db.create_register(name, "CAD")?);
                selection.session = None;
                selection.before_session = None;
            }
            Operation::Open {
                register,
                date,
                opening,
            } => {
                selection.register = Some(*register);
                selection.session = Some(self.db.open_session(*register, date, *opening)?);
                selection.before_entry = None;
                selection.before_session = None;
            }
            Operation::Record {
                session,
                direction,
                amount,
                reason,
                request_key,
            } => {
                self.db.record_entries(
                    *session,
                    &[NewEntry {
                        direction: *direction,
                        amount: *amount,
                        reason,
                        request_key,
                    }],
                )?;
                selection.session = Some(*session);
                selection.direction = *direction;
                selection.before_entry = None;
            }
            Operation::Close {
                session,
                counted,
                expected,
            } => self
                .db
                .close_session_if_unchanged(*session, *counted, *expected)?,
        }
        Ok(())
    }

    fn load(&self, selection: &Selection) -> crate::database::Result<Snapshot> {
        let registers = self.db.registers()?;
        let register = registers
            .iter()
            .find(|register| Some(register.id) == selection.register)
            .or_else(|| registers.first())
            .cloned();
        let mut sessions = if let Some(register) = &register {
            self.db
                .sessions(register.id, selection.before_session, PAGE_SIZE + 1)?
        } else {
            Vec::new()
        };
        let more_sessions = sessions.len() > PAGE_SIZE as usize;
        sessions.truncate(PAGE_SIZE as usize);
        let active_session = register
            .as_ref()
            .map(|register| self.db.active_session(register.id))
            .transpose()?
            .flatten();
        let session = if let Some(id) = selection.session {
            let session = self.db.session(id)?;
            if Some(session.register_id) != register.as_ref().map(|register| register.id) {
                return Err(crate::database::Error::InvalidInput(
                    "Session belongs to another register",
                ));
            }
            Some(session)
        } else {
            active_session.clone().or_else(|| sessions.first().cloned())
        };
        let balance = session
            .as_ref()
            .map(|session| self.db.balance(session.id))
            .transpose()?;
        let mut entries = if let Some(session) = &session {
            self.db.recent_entries(
                session.id,
                selection.direction,
                selection.before_entry,
                PAGE_SIZE + 1,
            )?
        } else {
            Vec::new()
        };
        let more_entries = entries.len() > PAGE_SIZE as usize;
        entries.truncate(PAGE_SIZE as usize);
        Ok(Snapshot {
            registers,
            register,
            sessions,
            session,
            balance,
            entries,
            more_entries,
            more_sessions,
            active_session,
        })
    }
}

fn friendly_error(error: &crate::database::Error) -> String {
    match error {
        crate::database::Error::Sqlite(rusqlite::Error::SqliteFailure(details, _)) => {
            match details.code {
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked =>
                    "The register is busy. Wait a moment, then retry.".into(),
                rusqlite::ErrorCode::ConstraintViolation =>
                    "This change could not be saved. Check the amount, date, and whether the day is already open or closed.".into(),
                _ => "Could not save or load the register. Check that the database location is available, then retry.".into(),
            }
        }
        _ => error.to_string(),
    }
}
