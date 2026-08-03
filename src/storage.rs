//! `SQLite` persistence. SQL details and row representations do not escape this module.

pub mod connection;
pub mod migration;
pub mod repository;
pub mod worker;

#[allow(unused_imports)]
pub use connection::open_primary;
#[allow(unused_imports)]
pub use migration::{LATEST_SCHEMA_VERSION, MigrationError, migrate};
