//! `SQLite` database startup and capability checks.
//!
//! This module owns only the connection-startup boundary. It does not create
//! application tables, run migrations, or expose a raw connection to callers.
//! The `rusqlite` dependency must be built with its `bundled` feature so the
//! application uses the tested `SQLite` amalgamation rather than an arbitrary
//! system library.

use std::{
    error::Error,
    fmt,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{Connection, Transaction};

/// The minimum `SQLite` version accepted by the local storage boundary.
///
/// The bundled `SQLite` build used by the workspace must be pinned to this
/// version or a newer one before a database is opened. Keeping this check at
/// runtime makes an accidentally different native `SQLite` library fail closed.
pub const MINIMUM_SQLITE_VERSION: SqliteVersion = SqliteVersion::new(3, 51, 3);

/// A parsed `SQLite` semantic version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SqliteVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl SqliteVersion {
    /// Creates a `SQLite` version value.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the major component.
    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the minor component.
    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Returns the patch component.
    #[must_use]
    pub const fn patch(self) -> u32 {
        self.patch
    }

    fn parse(raw: &str) -> Result<Self, DatabaseError> {
        let mut components = raw.split('.');
        let major = parse_version_component(components.next(), raw)?;
        let minor = parse_version_component(components.next(), raw)?;
        let patch = parse_version_component(components.next(), raw)?;

        if components.next().is_some() {
            return Err(DatabaseError::InvalidSqliteVersion {
                value: raw.to_owned(),
            });
        }

        Ok(Self::new(major, minor, patch))
    }
}

impl fmt::Display for SqliteVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_version_component(component: Option<&str>, raw: &str) -> Result<u32, DatabaseError> {
    component
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| DatabaseError::InvalidSqliteVersion {
            value: raw.to_owned(),
        })
}

/// Capabilities confirmed for an opened local database connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseCapabilities {
    sqlite_version: SqliteVersion,
    fts5: bool,
    wal: bool,
}

impl DatabaseCapabilities {
    /// Returns the `SQLite` version reported by the opened connection.
    #[must_use]
    pub const fn sqlite_version(self) -> SqliteVersion {
        self.sqlite_version
    }

    /// Returns whether FTS5 was confirmed on this connection.
    #[must_use]
    pub const fn fts5(self) -> bool {
        self.fts5
    }

    /// Returns whether WAL mode was confirmed on this connection.
    #[must_use]
    pub const fn wal(self) -> bool {
        self.wal
    }
}

/// A local `SQLite` connection that passed startup capability checks.
pub struct Database {
    connection: Mutex<Connection>,
    capabilities: DatabaseCapabilities,
}

impl Database {
    /// Opens a local database and configures its required capabilities.
    ///
    /// This method performs no schema or migration work. The connection is
    /// rejected unless it reports at least [`MINIMUM_SQLITE_VERSION`], can
    /// create an FTS5 virtual table in the temporary schema, and accepts WAL
    /// journal mode. The returned handle is intentionally opaque; write
    /// serialization belongs to the storage boundary added in a later batch.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError`] when opening or capability checks fail.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DatabaseError> {
        let connection = Connection::open(path).map_err(|source| DatabaseError::Open { source })?;

        let sqlite_version = query_sqlite_version(&connection)?;
        if sqlite_version < MINIMUM_SQLITE_VERSION {
            return Err(DatabaseError::UnsupportedSqliteVersion {
                found: sqlite_version,
                minimum: MINIMUM_SQLITE_VERSION,
            });
        }

        if !check_fts5(&connection)? {
            return Err(DatabaseError::Fts5Unavailable);
        }

        if !configure_wal(&connection)? {
            return Err(DatabaseError::WalUnavailable);
        }

        Ok(Self {
            connection: Mutex::new(connection),
            capabilities: DatabaseCapabilities {
                sqlite_version,
                fts5: true,
                wal: true,
            },
        })
    }

    /// Returns the capabilities confirmed during [`Self::open`].
    #[must_use]
    pub const fn capabilities(&self) -> DatabaseCapabilities {
        self.capabilities
    }

    /// Executes one write operation while holding the database write lock.
    ///
    /// The closure receives a shared `SQLite` connection because `rusqlite`
    /// exposes mutation methods through `&Connection`. The mutex guarantees
    /// that only one such operation runs at a time; callers must keep the
    /// closure bounded and must not perform blocking work inside it.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::WriteLockPoisoned`] if a previous writer
    /// panicked, or [`DatabaseError::WriteOperation`] when `SQLite` rejects the
    /// operation.
    pub fn execute_write<T, F>(&self, operation: F) -> Result<T, DatabaseError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T>,
    {
        let connection = self.lock_connection()?;
        operation(&connection).map_err(|source| DatabaseError::WriteOperation { source })
    }

    /// Executes and commits one bounded write transaction under the write lock.
    ///
    /// If the closure returns an error, the transaction is dropped and `SQLite`
    /// rolls it back. The commit is attempted only after the closure succeeds.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::WriteLockPoisoned`] if a previous writer
    /// panicked, [`DatabaseError::WriteTransaction`] for an operation failure,
    /// or [`DatabaseError::TransactionCommit`] when commit fails.
    pub fn with_write_transaction<T, F>(&self, operation: F) -> Result<T, DatabaseError>
    where
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    {
        let mut connection = self.lock_connection()?;
        let transaction = connection
            .transaction()
            .map_err(|source| DatabaseError::WriteTransaction { source })?;
        let value =
            operation(&transaction).map_err(|source| DatabaseError::WriteTransaction { source })?;
        transaction
            .commit()
            .map_err(|source| DatabaseError::TransactionCommit { source })?;
        Ok(value)
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, DatabaseError> {
        self.connection
            .lock()
            .map_err(|_| DatabaseError::WriteLockPoisoned)
    }

    /// Applies migrations through the database's sole mutable write boundary.
    ///
    /// The connection remains private. A caller must hold exclusive access to
    /// this `Database` value for the duration of migration work, which keeps
    /// writes serialized without imposing an async runtime on the storage
    /// crate.
    ///
    /// # Errors
    ///
    /// Returns [`crate::migration::MigrationError`] when definitions are
    /// invalid, the ledger is inconsistent, or `SQLite` rejects a statement.
    pub fn apply_migrations(
        &mut self,
        migrations: &[crate::migration::Migration],
    ) -> Result<(), crate::migration::MigrationError> {
        let connection = self.connection.get_mut().map_err(|_| {
            crate::migration::MigrationError::InvalidDefinition("database write lock is poisoned")
        })?;
        crate::migration::apply_migrations(connection, migrations)
    }
}

fn query_sqlite_version(connection: &Connection) -> Result<SqliteVersion, DatabaseError> {
    let raw: String = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|source| DatabaseError::VersionQuery { source })?;

    SqliteVersion::parse(&raw)
}

fn check_fts5(connection: &Connection) -> Result<bool, DatabaseError> {
    let compile_option_available: i64 = connection
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .map_err(|source| DatabaseError::Fts5Check { source })?;

    if compile_option_available == 0 {
        return Ok(false);
    }

    connection
        .execute_batch(
            "CREATE VIRTUAL TABLE temp.aurorix_fts5_capability_check USING fts5(content);\
             DROP TABLE temp.aurorix_fts5_capability_check;",
        )
        .map_err(|source| DatabaseError::Fts5Check { source })?;

    Ok(true)
}

fn configure_wal(connection: &Connection) -> Result<bool, DatabaseError> {
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|source| DatabaseError::WalCheck { source })?;

    Ok(journal_mode.eq_ignore_ascii_case("wal"))
}

/// Startup failures for the local `SQLite` boundary.
///
/// Display text is deliberately stable and excludes local paths, SQL text,
/// and driver error bodies. Implementations may inspect [`Error::source`] for
/// private diagnostics without promoting those details to a public contract.
#[derive(Debug)]
pub enum DatabaseError {
    /// The `SQLite` connection could not be opened.
    Open { source: rusqlite::Error },
    /// Reading the `SQLite` version failed.
    VersionQuery { source: rusqlite::Error },
    /// The `SQLite` version string was not a three-component numeric version.
    InvalidSqliteVersion { value: String },
    /// The `SQLite` version is older than the tested minimum.
    UnsupportedSqliteVersion {
        /// Version reported by `SQLite`.
        found: SqliteVersion,
        /// Minimum version accepted by this boundary.
        minimum: SqliteVersion,
    },
    /// The build does not provide FTS5.
    Fts5Unavailable,
    /// Checking or exercising FTS5 failed unexpectedly.
    Fts5Check { source: rusqlite::Error },
    /// WAL mode could not be confirmed for this database.
    WalUnavailable,
    /// Configuring WAL failed unexpectedly.
    WalCheck { source: rusqlite::Error },
    /// A previous writer panicked while holding the connection lock.
    WriteLockPoisoned,
    /// A non-transactional write operation failed.
    WriteOperation { source: rusqlite::Error },
    /// A transaction could not be started or its operation failed.
    WriteTransaction { source: rusqlite::Error },
    /// A write transaction could not be committed.
    TransactionCommit { source: rusqlite::Error },
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { .. } => formatter.write_str("unable to open local database"),
            Self::VersionQuery { .. } => formatter.write_str("unable to read SQLite version"),
            Self::InvalidSqliteVersion { .. } => {
                formatter.write_str("SQLite reported an invalid version")
            }
            Self::UnsupportedSqliteVersion { .. } => {
                formatter.write_str("SQLite version is not supported")
            }
            Self::Fts5Unavailable => formatter.write_str("SQLite FTS5 is unavailable"),
            Self::Fts5Check { .. } => formatter.write_str("unable to check SQLite FTS5"),
            Self::WalUnavailable => formatter.write_str("SQLite WAL mode is unavailable"),
            Self::WalCheck { .. } => formatter.write_str("unable to configure SQLite WAL mode"),
            Self::WriteLockPoisoned => formatter.write_str("SQLite write lock is unavailable"),
            Self::WriteOperation { .. } => formatter.write_str("SQLite write operation failed"),
            Self::WriteTransaction { .. } => formatter.write_str("SQLite write transaction failed"),
            Self::TransactionCommit { .. } => {
                formatter.write_str("SQLite write transaction commit failed")
            }
        }
    }
}

impl Error for DatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open { source }
            | Self::VersionQuery { source }
            | Self::Fts5Check { source }
            | Self::WalCheck { source }
            | Self::WriteOperation { source }
            | Self::WriteTransaction { source }
            | Self::TransactionCommit { source } => Some(source),
            Self::InvalidSqliteVersion { .. }
            | Self::UnsupportedSqliteVersion { .. }
            | Self::Fts5Unavailable
            | Self::WalUnavailable
            | Self::WriteLockPoisoned => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        Database, DatabaseCapabilities, DatabaseError, MINIMUM_SQLITE_VERSION, SqliteVersion,
    };
    use crate::migration::Migration;

    static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    fn temporary_database_path() -> PathBuf {
        let sequence = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "aurorix-storage-database-{}-{sequence}.sqlite",
            std::process::id()
        ))
    }

    #[test]
    fn sqlite_version_is_ordered_and_displayed() {
        let version = SqliteVersion::new(3, 51, 3);

        assert_eq!(version, MINIMUM_SQLITE_VERSION);
        assert_eq!(version.to_string(), "3.51.3");
        assert_eq!(version.major(), 3);
        assert_eq!(version.minor(), 51);
        assert_eq!(version.patch(), 3);
    }

    #[test]
    fn database_opens_with_required_capabilities() {
        let path = temporary_database_path();
        let database = Database::open(&path).expect("bundled SQLite capabilities");
        let capabilities: DatabaseCapabilities = database.capabilities();

        assert!(capabilities.sqlite_version() >= MINIMUM_SQLITE_VERSION);
        assert!(capabilities.fts5());
        assert!(capabilities.wal());

        drop(database);
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn in_memory_database_reports_wal_unavailable() {
        let Err(error) = Database::open(":memory:") else {
            panic!("in-memory SQLite cannot use WAL");
        };

        assert!(matches!(error, DatabaseError::WalUnavailable));
        assert_eq!(error.to_string(), "SQLite WAL mode is unavailable");
    }

    #[test]
    fn migrations_use_the_private_database_write_boundary() {
        let path = temporary_database_path();
        let mut database = Database::open(&path).expect("bundled SQLite capabilities");
        database
            .apply_migrations(&[Migration {
                version: 1,
                name: "create_marker",
                sql: "CREATE TABLE marker (value TEXT NOT NULL)",
            }])
            .expect("migration succeeds through database boundary");

        drop(database);
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn write_operations_are_committed_in_call_order() {
        let path = temporary_database_path();
        let database = Database::open(&path).expect("bundled SQLite capabilities");
        database
            .execute_write(|connection| {
                connection.execute("CREATE TABLE events (value INTEGER NOT NULL)", [])
            })
            .expect("create table");

        database
            .with_write_transaction(|transaction| {
                transaction.execute("INSERT INTO events (value) VALUES (1)", [])?;
                transaction.execute("INSERT INTO events (value) VALUES (2)", [])?;
                Ok(())
            })
            .expect("transaction succeeds");

        let values = database
            .execute_write(|connection| {
                let mut statement =
                    connection.prepare("SELECT value FROM events ORDER BY rowid")?;
                let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("read values through write boundary");
        assert_eq!(values, vec![1, 2]);

        drop(database);
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn failed_write_transaction_rolls_back_all_statements() {
        let path = temporary_database_path();
        let database = Database::open(&path).expect("bundled SQLite capabilities");
        database
            .execute_write(|connection| {
                connection.execute("CREATE TABLE events (value INTEGER NOT NULL)", [])
            })
            .expect("create table");

        let error = database
            .with_write_transaction(|transaction| {
                transaction.execute("INSERT INTO events (value) VALUES (1)", [])?;
                Err::<(), _>(rusqlite::Error::InvalidQuery)
            })
            .expect_err("transaction must fail");
        assert!(matches!(error, DatabaseError::WriteTransaction { .. }));

        let count = database
            .execute_write(|connection| {
                connection.query_row("SELECT COUNT(*) FROM events", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .expect("count rows");
        assert_eq!(count, 0);

        drop(database);
        fs::remove_file(path).expect("remove temporary database");
    }
}
