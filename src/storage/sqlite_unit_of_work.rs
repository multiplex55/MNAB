//! Transaction boundary used by the storage worker. Dropping either type without committing uses
//! rusqlite's rollback-on-drop behavior, including validation, audit, and command failures.
use super::{
    repository::{UnitOfWork, UnitOfWorkFactory},
    sqlite_repositories::SqliteRepositories,
};
use crate::error::RepositoryError;
use rusqlite::Connection;

pub struct SqliteUnitOfWorkFactory<'connection> {
    connection: &'connection mut Connection,
}
impl<'connection> SqliteUnitOfWorkFactory<'connection> {
    #[must_use]
    pub const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }
}
pub struct SqliteUnitOfWork<'connection> {
    repositories: SqliteRepositories<'connection>,
}
impl UnitOfWorkFactory for SqliteUnitOfWorkFactory<'_> {
    type Work<'a>
        = SqliteUnitOfWork<'a>
    where
        Self: 'a;
    fn begin(&mut self) -> Result<Self::Work<'_>, RepositoryError> {
        let transaction = self.connection.transaction().map_err(repo)?;
        Ok(SqliteUnitOfWork {
            repositories: SqliteRepositories::new(transaction),
        })
    }
}
impl<'connection> UnitOfWork for SqliteUnitOfWork<'connection> {
    type Repositories = SqliteRepositories<'connection>;
    fn repositories(&mut self) -> &mut Self::Repositories {
        &mut self.repositories
    }
    fn commit(self) -> Result<(), RepositoryError> {
        self.repositories.commit()
    }
    fn rollback(self) -> Result<(), RepositoryError> {
        self.repositories.rollback()
    }
}
fn repo<E: std::error::Error + Send + Sync + 'static>(source: E) -> RepositoryError {
    RepositoryError::Failed {
        source: Box::new(source),
    }
}
