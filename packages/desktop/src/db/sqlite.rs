use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::fallible_iterator::FallibleIterator as _;
use rusqlite::types::ValueRef;
use rusqlite::{Batch, Connection, InterruptHandle, OpenFlags, Statement};

use super::sql::quote_literal;
use super::{
    Column, ColumnInfo, ObjectKind, Outcome, ResultSet, StatementResult, TableInfo, TableRef,
    Value, ROW_LIMIT,
};

#[derive(Clone)]
pub struct SqliteSession {
    connection: Arc<Mutex<Connection>>,
    interrupt: Arc<InterruptHandle>,
}

impl fmt::Debug for SqliteSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SqliteSession")
            .finish_non_exhaustive()
    }
}

impl SqliteSession {
    pub async fn open(path: String, read_only: bool) -> Result<Self, String> {
        let path = expand_home(&path);
        tokio::task::spawn_blocking(move || {
            let access = if read_only {
                OpenFlags::SQLITE_OPEN_READ_ONLY
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            };
            let flags = access | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let connection = Connection::open_with_flags(&path, flags)
                .map_err(|error| format!("cannot open {path}: {error}"))?;
            connection
                .busy_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            // SQLite opens lazily; reading the schema is what rejects a file
            // that is not a database.
            connection
                .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
                .map_err(|error| format!("cannot open {path}: {error}"))?;
            Ok(Self {
                interrupt: Arc::new(connection.get_interrupt_handle()),
                connection: Arc::new(Mutex::new(connection)),
            })
        })
        .await
        .map_err(join_error)?
    }

    async fn blocking<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Connection) -> T + Send + 'static,
    ) -> Result<T, String> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || operation(&connection.lock().unwrap()))
            .await
            .map_err(join_error)
    }

    pub async fn run(&self, sql: String) -> Vec<StatementResult> {
        match self.blocking(move |connection| run(connection, &sql)).await {
            Ok(results) => results,
            Err(message) => vec![StatementResult {
                sql: String::new(),
                outcome: Outcome::Error {
                    message,
                    position: None,
                },
                elapsed: Duration::ZERO,
            }],
        }
    }

    pub async fn tables(&self) -> Result<Vec<TableInfo>, String> {
        self.blocking(|connection| tables(connection).map_err(|error| error.to_string()))
            .await?
    }

    pub async fn version(&self) -> Result<String, String> {
        self.blocking(|connection| {
            connection
                .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))
                .map(|version| format!("SQLite {version}"))
                .map_err(|error| error.to_string())
        })
        .await?
    }

    pub fn cancel(&self) {
        self.interrupt.interrupt();
    }

    pub async fn in_transaction(&self) -> bool {
        self.blocking(|connection| !connection.is_autocommit())
            .await
            .unwrap_or(false)
    }

    /// Prepares each statement in `sql` without running it, and returns the
    /// first error with its byte offset. Skipped while a statement runs.
    // ponytail: a statement that needs an earlier one in `sql` to have run
    // (CREATE TABLE t, then INSERT INTO t) reports "no such table"; check
    // syntax only if that turns out to be common.
    pub async fn check(&self, sql: String) -> Option<(String, usize)> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.try_lock().ok()?;
            let mut batch = Batch::new(&connection, &sql);
            loop {
                match batch.next() {
                    Ok(Some(_)) => {}
                    Ok(None) => return None,
                    Err(error) => {
                        return match failure(error, &sql) {
                            Outcome::Error { message, position } => {
                                Some((message, position.unwrap_or(0)))
                            }
                            _ => None,
                        }
                    }
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    /// Runs one statement that must not write, and returns up to `limit` rows
    /// as JSON objects, plus whether more rows existed.
    pub async fn select_json(
        &self,
        sql: String,
        limit: usize,
    ) -> Result<(Vec<String>, bool), String> {
        self.blocking(move |connection| select_json(connection, &sql, limit))
            .await?
    }

    /// Runs one statement and returns the rows it changed.
    pub async fn execute_one(&self, sql: String) -> Result<u64, String> {
        self.blocking(move |connection| {
            // `prepare` refuses a second statement after the first.
            let mut statement = connection.prepare(&sql)?;
            statement.execute([]).map(|count| count as u64)
        })
        .await?
        .map_err(|error| error.to_string())
    }
}

fn select_json(
    connection: &Connection,
    sql: &str,
    limit: usize,
) -> Result<(Vec<String>, bool), String> {
    // `prepare` refuses a second statement after the first.
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    if !statement.readonly() {
        return Err("select runs read-only statements only".to_string());
    }
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| serde_json::Value::from(*name).to_string())
        .collect();
    let mut rows = statement.raw_query();
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        if out.len() == limit {
            return Ok((out, true));
        }
        let mut fields = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index).map_err(|error| error.to_string())? {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Integer(number) => number.into(),
                ValueRef::Real(number) => number.into(),
                other => text(other).into(),
            };
            fields.push(format!("{name}:{value}"));
        }
        out.push(format!("{{{}}}", fields.join(",")));
    }
    Ok((out, false))
}

/// `~/...` as a path under the home folder; SQLite takes `~` literally.
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

fn join_error(error: tokio::task::JoinError) -> String {
    format!("database task join error: {error}")
}

fn run(connection: &Connection, sql: &str) -> Vec<StatementResult> {
    let mut results = Vec::new();
    let mut batch = Batch::new(connection, sql);
    loop {
        let started = Instant::now();
        let (text, outcome) = match batch.next() {
            Ok(None) => break,
            Ok(Some(mut statement)) => (
                statement.expanded_sql().unwrap_or_default(),
                execute(connection, &mut statement).unwrap_or_else(|error| failure(error, sql)),
            ),
            Err(error) => (String::new(), failure(error, sql)),
        };
        let failed = matches!(outcome, Outcome::Error { .. });
        results.push(StatementResult {
            sql: text,
            outcome,
            elapsed: started.elapsed(),
        });
        // SQLite cannot skip past a statement it failed to parse.
        if failed {
            break;
        }
    }
    results
}

fn failure(error: rusqlite::Error, sql: &str) -> Outcome {
    match error {
        // `tail` is the unparsed rest of `sql`; the offset counts from its start.
        rusqlite::Error::SqlInputError {
            msg,
            sql: tail,
            offset,
            ..
        } => Outcome::Error {
            message: msg,
            position: usize::try_from(offset)
                .ok()
                .map(|offset| sql.len().saturating_sub(tail.len()) + offset),
        },
        other => Outcome::Error {
            message: other.to_string(),
            position: None,
        },
    }
}

fn execute(connection: &Connection, statement: &mut Statement<'_>) -> rusqlite::Result<Outcome> {
    let columns: Vec<Column> = statement
        .columns()
        .iter()
        .map(|column| {
            let type_name = column.decl_type().unwrap_or_default().to_string();
            Column {
                name: column.name().to_string(),
                numeric: has_numeric_affinity(&type_name),
                type_name,
            }
        })
        .collect();
    if columns.is_empty() {
        // `changes()` keeps its old value across DDL, so count the difference.
        let before = connection.total_changes();
        statement.raw_execute()?;
        return Ok(Outcome::Affected(connection.total_changes() - before));
    }

    let mut result = ResultSet {
        columns,
        ..ResultSet::default()
    };
    // Expression columns declare no type: call one numeric when every value
    // it holds is a number.
    let mut numbers = vec![0_usize; result.columns.len()];
    let mut others = vec![0_usize; result.columns.len()];
    let mut rows = statement.raw_query();
    while let Some(row) = rows.next()? {
        if result.rows.len() == ROW_LIMIT {
            result.truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(result.columns.len());
        for index in 0..result.columns.len() {
            let value = row.get_ref(index)?;
            match value {
                ValueRef::Integer(_) | ValueRef::Real(_) => numbers[index] += 1,
                ValueRef::Null => {}
                ValueRef::Text(_) | ValueRef::Blob(_) => others[index] += 1,
            }
            values.push(text(value));
        }
        result.rows.push(values);
    }
    for (index, column) in result.columns.iter_mut().enumerate() {
        if column.type_name.is_empty() {
            column.numeric = numbers[index] > 0 && others[index] == 0;
        }
    }
    Ok(Outcome::Rows(result))
}

/// SQLite's own rule for which declared types hold numbers.
fn has_numeric_affinity(type_name: &str) -> bool {
    let upper = type_name.to_uppercase();
    ["INT", "REAL", "FLOA", "DOUB", "NUM", "DEC"]
        .iter()
        .any(|part| upper.contains(part))
}

fn text(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(number) => Some(number.to_string()),
        ValueRef::Real(number) => Some(number.to_string()),
        ValueRef::Text(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => Some(format!("<{} bytes>", bytes.len())),
    }
}

pub fn structure_queries(table: &TableRef) -> Vec<(&'static str, String)> {
    let name = quote_literal(&Some(table.name.clone()));
    vec![
        (
            "Columns",
            format!(
                "SELECT name, type, \"notnull\" AS not_null, dflt_value AS \"default\", pk \
                 FROM pragma_table_info({name})"
            ),
        ),
        (
            "Indexes",
            format!("SELECT name, \"unique\", origin, partial FROM pragma_index_list({name})"),
        ),
        (
            "Foreign keys",
            format!(
                "SELECT \"from\", \"table\", \"to\", on_update, on_delete \
                 FROM pragma_foreign_key_list({name})"
            ),
        ),
        (
            "Definition",
            format!("SELECT sql FROM sqlite_master WHERE name = {name}"),
        ),
    ]
}

fn tables(connection: &Connection) -> rusqlite::Result<Vec<TableInfo>> {
    let mut statement = connection.prepare(
        "SELECT m.name, m.type, p.name, p.type, p.\"notnull\", p.dflt_value, p.pk \
         FROM sqlite_master m, pragma_table_info(m.name) p \
         WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%' \
         ORDER BY m.name, p.cid",
    )?;
    let mut tables: Vec<TableInfo> = Vec::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let column = ColumnInfo {
            name: row.get(2)?,
            type_name: row.get(3)?,
            nullable: row.get::<_, i64>(4)? == 0,
            default: row.get(5)?,
            primary_key: row.get::<_, i64>(6)? > 0,
        };
        match tables.last_mut() {
            Some(table) if table.table.name == name => table.columns.push(column),
            _ => tables.push(TableInfo {
                table: TableRef { schema: None, name },
                kind: if kind == "view" {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                columns: vec![column],
            }),
        }
    }
    Ok(tables)
}
