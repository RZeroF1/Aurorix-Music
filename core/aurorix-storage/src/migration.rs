//! Transactional, append-only schema migrations.
//!
//! Migrations are application-owned source constants. Once a migration has
//! been applied, changing its number, name, or SQL changes its checksum and is
//! rejected on the next open rather than silently rewriting the database.

use rusqlite::{Connection, Transaction, params};
use sha2::{Digest, Sha256};
use std::{error::Error, fmt};

const LEDGER_SQL: &str = "CREATE TABLE IF NOT EXISTS aurorix_schema_migrations (version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, checksum TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)";

/// One immutable schema migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Monotonically increasing migration number.
    pub version: i64,
    /// Stable human-readable migration name.
    pub name: &'static str,
    /// SQL executed inside one transaction.
    pub sql: &'static str,
}

impl Migration {
    fn checksum(self) -> String {
        let mut digest = Sha256::new();
        digest.update(self.version.to_string().as_bytes());
        digest.update([0]);
        digest.update(self.name.as_bytes());
        digest.update([0]);
        digest.update(self.sql.as_bytes());
        format!("{:x}", digest.finalize())
    }
}

/// Failure while validating or applying migrations.
#[derive(Debug)]
pub enum MigrationError {
    /// The migration list is not strictly increasing or contains duplicates.
    InvalidDefinition(&'static str),
    /// A previously applied migration no longer matches its source checksum.
    ChecksumMismatch { version: i64, name: String },
    /// The database contains a migration newer than this binary understands.
    UnknownDatabaseVersion { version: i64 },
    /// `SQLite` reported an error.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition(message) => {
                write!(formatter, "invalid migration definition: {message}")
            }
            Self::ChecksumMismatch { version, name } => {
                write!(
                    formatter,
                    "migration {version} ({name}) checksum does not match"
                )
            }
            Self::UnknownDatabaseVersion { version } => write!(
                formatter,
                "database migration version {version} is newer than this binary"
            ),
            Self::Sqlite(error) => write!(formatter, "SQLite migration failed: {error}"),
        }
    }
}

impl Error for MigrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for MigrationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Applies all pending migrations and validates the existing ledger.
///
/// The caller supplies the complete, ordered migration list. The ledger table
/// is created before any migration and each pending migration is committed in
/// its own transaction. An error rolls back that migration and leaves the
/// database at its previous version.
///
/// # Errors
///
/// Returns [`MigrationError`] when definitions are invalid, an existing
/// ledger entry no longer matches its checksum, the database is newer than the
/// binary, or `SQLite` rejects a statement.
pub fn apply_migrations(
    connection: &mut Connection,
    migrations: &[Migration],
) -> Result<(), MigrationError> {
    validate_definitions(migrations)?;
    connection.execute_batch(LEDGER_SQL)?;

    let applied = read_applied(connection)?;
    let highest_known = migrations.last().map_or(0, |migration| migration.version);
    if let Some((version, _, _)) = applied.last()
        && *version > highest_known
    {
        return Err(MigrationError::UnknownDatabaseVersion { version: *version });
    }

    for (version, name, checksum) in &applied {
        let Some(migration) = migrations
            .iter()
            .find(|candidate| candidate.version == *version)
        else {
            return Err(MigrationError::UnknownDatabaseVersion { version: *version });
        };
        if migration.name != name || migration.checksum() != *checksum {
            return Err(MigrationError::ChecksumMismatch {
                version: *version,
                name: name.clone(),
            });
        }
    }

    for migration in migrations {
        if applied
            .iter()
            .any(|(version, _, _)| *version == migration.version)
        {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        record_migration(&transaction, *migration)?;
        transaction.commit()?;
    }

    Ok(())
}

fn validate_definitions(migrations: &[Migration]) -> Result<(), MigrationError> {
    let mut previous = 0;
    for migration in migrations {
        if migration.version <= 0 || migration.version <= previous {
            return Err(MigrationError::InvalidDefinition(
                "versions must be positive and strictly increasing",
            ));
        }
        if migration.name.trim().is_empty() || migration.sql.trim().is_empty() {
            return Err(MigrationError::InvalidDefinition(
                "name and SQL must not be empty",
            ));
        }
        previous = migration.version;
    }
    Ok(())
}

fn read_applied(connection: &Connection) -> Result<Vec<(i64, String, String)>, MigrationError> {
    let mut statement = connection.prepare(
        "SELECT version, name, checksum FROM aurorix_schema_migrations ORDER BY version",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(MigrationError::from)
}

fn record_migration(
    transaction: &Transaction<'_>,
    migration: Migration,
) -> Result<(), MigrationError> {
    transaction.execute(
        "INSERT INTO aurorix_schema_migrations (version, name, checksum) VALUES (?1, ?2, ?3)",
        params![migration.version, migration.name, migration.checksum()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Migration, MigrationError, apply_migrations};
    use rusqlite::Connection;

    const FIRST: Migration = Migration {
        version: 1,
        name: "create_marker",
        sql: "CREATE TABLE marker (value TEXT NOT NULL)",
    };
    const SECOND: Migration = Migration {
        version: 2,
        name: "create_second_marker",
        sql: "CREATE TABLE second_marker (value INTEGER NOT NULL)",
    };

    #[test]
    fn applies_pending_migrations_and_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &[FIRST, SECOND]).unwrap();
        apply_migrations(&mut connection, &[FIRST, SECOND]).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM aurorix_schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        assert!(table_exists(&connection, "marker"));
    }

    #[test]
    fn rejects_changed_sql_after_application() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &[FIRST]).unwrap();
        let changed = Migration {
            sql: "CREATE TABLE marker (value INTEGER NOT NULL)",
            ..FIRST
        };
        assert!(matches!(
            apply_migrations(&mut connection, &[changed]),
            Err(MigrationError::ChecksumMismatch { version: 1, .. })
        ));
    }

    #[test]
    fn failed_migration_does_not_record_a_ledger_row() {
        let mut connection = Connection::open_in_memory().unwrap();
        let failing = Migration {
            version: 1,
            name: "fails",
            sql: "CREATE TABLE marker (",
        };
        assert!(apply_migrations(&mut connection, &[failing]).is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM aurorix_schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn rejects_unordered_migration_definitions() {
        let mut connection = Connection::open_in_memory().unwrap();
        let error = apply_migrations(&mut connection, &[SECOND, FIRST]).unwrap_err();
        assert!(matches!(error, MigrationError::InvalidDefinition(_)));
    }

    fn table_exists(connection: &Connection, name: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [name],
                |row| row.get(0),
            )
            .unwrap()
    }
}
