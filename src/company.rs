//! Local company configuration and password authentication, independent of ledger migrations.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand_core::OsRng;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyProfile {
    pub name: String,
    pub address: String,
    pub rules: String,
    pub closing_variance_threshold_cents: i64,
    /// Empty selects the operating system's default printer.
    pub printer_name: String,
    pub paper_size: String,
}

impl Default for CompanyProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            rules: String::new(),
            closing_variance_threshold_cents: 0,
            printer_name: String::new(),
            paper_size: "A4".into(),
        }
    }
}

/// Intentionally does not implement Debug to avoid exposing credentials in diagnostics.
pub struct SetupCompany {
    pub profile: CompanyProfile,
    pub username: String,
    pub password: String,
}

#[derive(Debug)]
pub enum CompanyError {
    Database(rusqlite::Error),
    InvalidInput(&'static str),
    AlreadyConfigured,
    NotConfigured,
    PasswordHash,
}

impl std::fmt::Display for CompanyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(_) => f.write_str("Company database operation failed"),
            Self::InvalidInput(message) => f.write_str(message),
            Self::AlreadyConfigured => f.write_str("Company setup is already complete"),
            Self::NotConfigured => f.write_str("Complete company setup first"),
            Self::PasswordHash => f.write_str("Password processing failed"),
        }
    }
}
impl std::error::Error for CompanyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            _ => None,
        }
    }
}
impl From<rusqlite::Error> for CompanyError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}
type Result<T> = std::result::Result<T, CompanyError>;

pub struct CompanyService {
    connection: Connection,
}

impl CompanyService {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // Separate additive tables leave the ledger's schema version and all existing data intact.
        connection.execute_batch("CREATE TABLE IF NOT EXISTS company_profile (
            id INTEGER PRIMARY KEY CHECK(id = 1), name TEXT NOT NULL, address TEXT NOT NULL,
            rules TEXT NOT NULL, variance_threshold INTEGER NOT NULL CHECK(variance_threshold >= 0),
            printer_name TEXT NOT NULL, paper_size TEXT NOT NULL CHECK(paper_size IN ('A4','Letter')));
            CREATE TABLE IF NOT EXISTS company_users (
            username TEXT PRIMARY KEY, password_hash TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS company_login_guard (
            id INTEGER PRIMARY KEY CHECK(id=1), failures INTEGER NOT NULL DEFAULT 0,
            locked_until INTEGER NOT NULL DEFAULT 0);
            INSERT OR IGNORE INTO company_login_guard(id) VALUES(1);")?;
        Ok(Self { connection })
    }

    pub fn profile(&self) -> Result<Option<CompanyProfile>> {
        Ok(self.connection.query_row("SELECT name,address,rules,variance_threshold,printer_name,paper_size FROM company_profile WHERE id=1", [], |r| {
            Ok(CompanyProfile { name:r.get(0)?, address:r.get(1)?, rules:r.get(2)?,
                closing_variance_threshold_cents:r.get(3)?, printer_name:r.get(4)?, paper_size:r.get(5)? })
        }).optional()?)
    }

    pub fn setup(&mut self, setup: SetupCompany) -> Result<()> {
        validate_profile(&setup.profile)?;
        let username = validate_username(&setup.username)?;
        validate_password(&setup.password)?;
        let password_hash = hash_password(&setup.password)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool =
            transaction.query_row("SELECT EXISTS(SELECT 1 FROM company_profile)", [], |r| {
                r.get(0)
            })?;
        if exists {
            return Err(CompanyError::AlreadyConfigured);
        }
        let p = &setup.profile;
        transaction.execute(
            "INSERT INTO company_profile VALUES(1,?1,?2,?3,?4,?5,?6)",
            params![
                p.name.trim(),
                p.address.trim(),
                p.rules.trim(),
                p.closing_variance_threshold_cents,
                p.printer_name.trim(),
                p.paper_size
            ],
        )?;
        transaction.execute(
            "INSERT INTO company_users(username,password_hash) VALUES(?1,?2)",
            params![username, password_hash],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<bool> {
        // Serialize verification with the persisted guard so concurrent app processes
        // cannot all pass the cooldown check before failed attempts are recorded.
        // The connection is private and no public method retains an open transaction.
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let authenticated = self.authenticate_inner(username, password)?;
        transaction.commit()?;
        Ok(authenticated)
    }

    fn authenticate_inner(&self, username: &str, password: &str) -> Result<bool> {
        let locked: bool = self.connection.query_row(
            "SELECT locked_until > unixepoch() FROM company_login_guard WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        if locked {
            return Ok(false);
        }
        if validate_username(username).is_err() || password.len() > 1024 {
            self.record_failed_login()?;
            return Ok(false);
        }
        let hash: Option<String> = self
            .connection
            .query_row(
                "SELECT password_hash FROM company_users WHERE username=?1",
                [username.trim()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(hash) = hash else {
            self.record_failed_login()?;
            return Ok(false);
        };
        let parsed = PasswordHash::new(&hash).map_err(|_| CompanyError::PasswordHash)?;
        match Argon2::default().verify_password(password.as_bytes(), &parsed) {
            Ok(()) => {
                self.connection.execute(
                    "UPDATE company_login_guard SET failures=0,locked_until=0 WHERE id=1",
                    [],
                )?;
                Ok(true)
            }
            Err(argon2::password_hash::Error::Password) => {
                self.record_failed_login()?;
                Ok(false)
            }
            Err(_) => Err(CompanyError::PasswordHash),
        }
    }

    fn record_failed_login(&self) -> Result<()> {
        // A global persisted cooldown also bounds attacks that rotate unknown usernames.
        self.connection.execute("UPDATE company_login_guard SET
            failures=CASE WHEN locked_until > 0 AND locked_until <= unixepoch() THEN 1 ELSE failures+1 END,
            locked_until=CASE WHEN failures >= 4 AND NOT(locked_until > 0 AND locked_until <= unixepoch()) THEN unixepoch()+30 ELSE 0 END WHERE id=1",[])?;
        Ok(())
    }

    /// The application must restrict this operation to its authenticated session.
    pub fn save_settings(&self, profile: &CompanyProfile) -> Result<()> {
        validate_profile(profile)?;
        let changed = self.connection.execute("UPDATE company_profile SET name=?1,address=?2,rules=?3,variance_threshold=?4,printer_name=?5,paper_size=?6 WHERE id=1",
            params![profile.name.trim(),profile.address.trim(),profile.rules.trim(),profile.closing_variance_threshold_cents,profile.printer_name.trim(),profile.paper_size])?;
        if changed == 0 {
            return Err(CompanyError::NotConfigured);
        }
        Ok(())
    }

    pub fn change_password(
        &self,
        username: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<bool> {
        validate_password(new_password)?;
        let hash = hash_password(new_password)?;
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if !self.authenticate_inner(username, current_password)? {
            transaction.commit()?;
            return Ok(false);
        }
        transaction.execute(
            "UPDATE company_users SET password_hash=?1 WHERE username=?2",
            params![hash, username.trim()],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}

fn validate_profile(p: &CompanyProfile) -> Result<()> {
    if p.name.chars().any(char::is_control)
        || p.printer_name.chars().any(char::is_control)
        || p.address
            .chars()
            .chain(p.rules.chars())
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        return Err(CompanyError::InvalidInput(
            "Company text contains unsupported control characters",
        ));
    }
    if p.name.trim().is_empty() || p.name.len() > 160 {
        return Err(CompanyError::InvalidInput(
            "Company name must contain 1–160 bytes",
        ));
    }
    if p.address.len() > 1000 || p.rules.len() > 4000 || p.printer_name.len() > 256 {
        return Err(CompanyError::InvalidInput(
            "Address, rules, or printer name is too long",
        ));
    }
    if !(0..=100_000_000).contains(&p.closing_variance_threshold_cents) {
        return Err(CompanyError::InvalidInput(
            "Variance threshold must be between 0 and 1,000,000",
        ));
    }
    if !matches!(p.paper_size.as_str(), "A4" | "Letter") {
        return Err(CompanyError::InvalidInput(
            "Paper size must be A4 or Letter",
        ));
    }
    Ok(())
}
fn validate_username(username: &str) -> Result<&str> {
    let username = username.trim();
    if username.is_empty()
        || username.len() > 64
        || !username
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'@'))
    {
        return Err(CompanyError::InvalidInput(
            "Username must contain 1–64 letters, numbers, dots, underscores, hyphens, or @",
        ));
    }
    Ok(username)
}
fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err(CompanyError::InvalidInput(
            "Password must contain at least 12 characters and at most 1024 bytes",
        ));
    }
    Ok(())
}
fn hash_password(password: &str) -> Result<String> {
    let mut salt_bytes = [0u8; 16];
    // Report entropy failures instead of panicking or generating a predictable salt.
    rand_core::RngCore::try_fill_bytes(&mut OsRng, &mut salt_bytes)
        .map_err(|_| CompanyError::PasswordHash)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| CompanyError::PasswordHash)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| CompanyError::PasswordHash)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> SetupCompany {
        SetupCompany {
            profile: CompanyProfile {
                name: "Example Company".into(),
                ..Default::default()
            },
            username: "admin".into(),
            password: "long test password".into(),
        }
    }
    #[test]
    fn setup_is_atomic_persistent_and_cannot_replace_existing_company() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.sqlite");
        let mut service = CompanyService::open(&path).unwrap();
        assert_eq!(service.profile().unwrap(), None);
        let mut invalid = setup();
        invalid.username = "invalid user".into();
        assert!(service.setup(invalid).is_err());
        assert_eq!(service.profile().unwrap(), None);
        service.setup(setup()).unwrap();
        assert!(matches!(
            service.setup(setup()),
            Err(CompanyError::AlreadyConfigured)
        ));
        drop(service);
        let service = CompanyService::open(path).unwrap();
        assert_eq!(service.profile().unwrap().unwrap().name, "Example Company");
        assert!(service.authenticate("admin", "long test password").unwrap());
        assert!(!service.authenticate("admin", "wrong password").unwrap());
        assert!(
            !service
                .authenticate("missing", "long test password")
                .unwrap()
        );
        let hash: String = service
            .connection
            .query_row("SELECT password_hash FROM company_users", [], |r| r.get(0))
            .unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(!hash.contains("long test password"));
    }
    #[test]
    fn settings_and_password_changes_validate_and_persist() {
        let mut service = CompanyService::open(":memory:").unwrap();
        assert!(matches!(
            service.save_settings(&setup().profile),
            Err(CompanyError::NotConfigured)
        ));
        service.setup(setup()).unwrap();
        let mut p = service.profile().unwrap().unwrap();
        p.name = "Updated".into();
        p.closing_variance_threshold_cents = 150;
        service.save_settings(&p).unwrap();
        assert_eq!(service.profile().unwrap(), Some(p.clone()));
        p.paper_size = "invalid".into();
        assert!(service.save_settings(&p).is_err());
        assert!(
            !service
                .change_password("admin", "wrong", "replacement password")
                .unwrap()
        );
        assert!(
            service
                .change_password("admin", "long test password", "replacement password")
                .unwrap()
        );
        assert!(!service.authenticate("admin", "long test password").unwrap());
        assert!(
            service
                .authenticate("admin", "replacement password")
                .unwrap()
        );
    }
    #[test]
    fn failed_logins_enforce_persisted_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("company.sqlite");
        let mut service = CompanyService::open(&path).unwrap();
        service.setup(setup()).unwrap();
        for _ in 0..5 {
            assert!(
                !service
                    .authenticate("missing", "incorrect password")
                    .unwrap()
            );
        }
        drop(service);
        let service = CompanyService::open(path).unwrap();
        assert!(!service.authenticate("admin", "long test password").unwrap());
        service
            .connection
            .execute(
                "UPDATE company_login_guard SET locked_until=unixepoch()-1 WHERE id=1",
                [],
            )
            .unwrap();
        assert!(service.authenticate("admin", "long test password").unwrap());
    }

    #[test]
    fn concurrent_login_failures_preserve_the_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("company.sqlite");
        let mut service = CompanyService::open(&path).unwrap();
        service.setup(setup()).unwrap();
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let service = CompanyService::open(path).unwrap();
                    assert!(
                        !service
                            .authenticate("missing", "incorrect password")
                            .unwrap()
                    );
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(!service.authenticate("admin", "long test password").unwrap());
    }
}
