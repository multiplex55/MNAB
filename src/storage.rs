//! `SQLite` persistence. SQL details and row representations do not escape this module.

pub mod connection;
pub mod diagnostics;
pub mod mapping;
pub mod migration;
pub mod model;
pub mod query_store;
pub mod repair;
pub mod repository;
pub mod sqlite_repositories;
pub mod sqlite_unit_of_work;
pub mod worker;

#[allow(unused_imports)]
pub use connection::open_primary;
#[allow(unused_imports)]
pub use migration::{LATEST_SCHEMA_VERSION, MigrationError, migrate};
