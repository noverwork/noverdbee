//! Sessions against the databases the user manages. The app's own data lives
//! in `crate::store`, not here.

mod postgres;
pub mod sql;
mod sqlite;
pub mod tunnel;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shared::DbConnectionRecord;

pub use sql::{TableRef, Value};

pub const DRIVER_SQLITE: &str = "sqlite";
pub const DRIVER_POSTGRES: &str = "postgres";

/// Rows a query in the editor may return before the result is cut short.
// ponytail: results are read into memory in one go; switch to a cursor and
// fetch on scroll if queries larger than this need to be browsed.
pub const ROW_LIMIT: usize = 10_000;

/// Rows a table tab loads at a time.
pub const PAGE_SIZE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// The declared type, empty when the database does not report one.
    pub type_name: String,
    /// Numbers align right in the grid.
    pub numeric: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    /// More rows existed than [`ROW_LIMIT`].
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Rows(ResultSet),
    Affected(u64),
    Error {
        message: String,
        /// Byte offset into the SQL that was run, when the database gives one.
        position: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementResult {
    pub sql: String,
    pub outcome: Outcome,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Table,
    View,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub table: TableRef,
    pub kind: ObjectKind,
    pub columns: Vec<ColumnInfo>,
}

/// An open connection to one database. Cloning shares the connection.
#[derive(Debug, Clone)]
pub enum Session {
    Sqlite(sqlite::SqliteSession),
    Postgres(Arc<postgres::PostgresSession>),
}

impl Session {
    pub async fn open(record: &DbConnectionRecord) -> Result<Self, String> {
        match record.driver.as_str() {
            DRIVER_SQLITE => sqlite::SqliteSession::open(record.path.clone(), record.read_only)
                .await
                .map(Session::Sqlite),
            DRIVER_POSTGRES => postgres::PostgresSession::open(record)
                .await
                .map(|session| Session::Postgres(Arc::new(session))),
            other => Err(format!("unsupported database driver: {other}")),
        }
    }

    /// Runs every statement in `sql` in order, stopping at the first error.
    pub async fn run(&self, sql: String) -> Vec<StatementResult> {
        match self {
            Session::Sqlite(session) => session.run(sql).await,
            Session::Postgres(session) => session.run(sql).await,
        }
    }

    /// Parses `sql` without running it; the first error and its byte offset.
    pub async fn check(&self, sql: String) -> Option<(String, usize)> {
        match self {
            Session::Sqlite(session) => session.check(sql).await,
            Session::Postgres(session) => session.check(sql).await,
        }
    }

    /// Every table and view with its columns.
    // ponytail: loads all columns up front; load them per table on expand if a
    // schema with thousands of tables makes connecting slow.
    pub async fn tables(&self) -> Result<Vec<TableInfo>, String> {
        match self {
            Session::Sqlite(session) => session.tables().await,
            Session::Postgres(session) => session.tables().await,
        }
    }

    /// Read-only queries that describe `table`, by title.
    pub fn structure_queries(&self, table: &TableRef) -> Vec<(&'static str, String)> {
        match self {
            Session::Sqlite(_) => sqlite::structure_queries(table),
            Session::Postgres(_) => postgres::structure_queries(table),
        }
    }

    /// The server version, for the Test connection result.
    pub async fn version(&self) -> Result<String, String> {
        match self {
            Session::Sqlite(session) => session.version().await,
            Session::Postgres(session) => session.version().await,
        }
    }

    /// Runs one read-only query; rows come back as JSON objects. Open a
    /// session for this alone: PostgreSQL leaves its read-only transaction open.
    pub async fn select_json(
        &self,
        sql: String,
        limit: usize,
    ) -> Result<(Vec<String>, bool), String> {
        match self {
            Session::Sqlite(session) => session.select_json(sql, limit).await,
            Session::Postgres(session) => session.select_json(&sql, limit).await,
        }
    }

    /// Runs exactly one statement and returns the rows it changed.
    pub async fn execute_one(&self, sql: String) -> Result<u64, String> {
        match self {
            Session::Sqlite(session) => session.execute_one(sql).await,
            Session::Postgres(session) => session.execute_one(&sql).await,
        }
    }

    /// A transaction the user opened is still open, perhaps failed.
    pub async fn in_transaction(&self) -> bool {
        match self {
            Session::Sqlite(session) => session.in_transaction().await,
            Session::Postgres(session) => session.in_transaction().await,
        }
    }

    /// The server ended the connection, or the network to it dropped.
    pub fn is_closed(&self) -> bool {
        match self {
            Session::Sqlite(_) => false,
            Session::Postgres(session) => session.is_closed(),
        }
    }

    /// Stops the statement that is running, if any.
    pub fn cancel(&self) {
        match self {
            Session::Sqlite(session) => session.cancel(),
            Session::Postgres(session) => session.cancel(),
        }
    }
}

/// The open sessions, by saved connection id.
#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<String, Session>>,
}

impl SessionManager {
    /// The open session for `record`, opening one if there is none or the
    /// one there was has closed.
    pub async fn connect(&self, record: &DbConnectionRecord) -> Result<Session, String> {
        if let Some(session) = self.get(&record.id) {
            if !session.is_closed() {
                return Ok(session);
            }
            tracing::info!(connection = %record.id, "Reconnecting a closed session");
        }
        let session = Session::open(record).await?;
        self.sessions
            .lock()
            .unwrap()
            .insert(record.id.clone(), session.clone());
        Ok(session)
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    pub fn disconnect(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }
}

#[cfg(test)]
mod tests;
