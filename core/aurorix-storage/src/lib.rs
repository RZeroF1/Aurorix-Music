//! `SQLite` implementation of application repositories, migrations, projections,
//! and the local outbox.

pub mod catalog_row;
pub mod catalog_schema;
pub mod database;
pub mod fts_schema;
pub mod migration;
pub mod scan_observation;
pub mod search;
pub mod stats_aggregator;
pub mod stats_schema;

/// The complete migration set for the current local catalog.
pub const LOCAL_MIGRATIONS: &[migration::Migration] = catalog_schema::LOCAL_CATALOG_MIGRATIONS;
