//! `SQLite` implementation of application repositories, migrations, projections,
//! and the local outbox.

pub mod catalog_row;
pub mod catalog_schema;
pub mod database;
pub mod fts_schema;
pub mod migration;
pub mod search;

/// The complete migration set for the current local catalog.
pub const LOCAL_MIGRATIONS: &[migration::Migration] = catalog_schema::LOCAL_CATALOG_MIGRATIONS;
