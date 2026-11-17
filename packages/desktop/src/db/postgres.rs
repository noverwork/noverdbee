use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use postgres_native_tls::MakeTlsConnector;
use shared::DbConnectionRecord;
use tokio_postgres::config::SslMode;
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::{CancelToken, Client, SimpleQueryMessage};

use super::sql::quote_literal;
use super::{
    Column, ColumnInfo, ObjectKind, Outcome, ResultSet, StatementResult, TableInfo, TableRef,
    ROW_LIMIT,
};

#[derive(Clone)]
pub struct PostgresSession {
    client: Arc<Client>,
    cancel: CancelToken,
    tls: MakeTlsConnector,
}

impl fmt::Debug for PostgresSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresSession")
            .finish_non_exhaustive()
    }
}

impl PostgresSession {
    pub async fn open(record: &DbConnectionRecord) -> Result<Self, String> {
        // ponytail: certificates are checked against the system roots only;
        // add a CA file and verify-full when a self-signed server needs them.
        let connector = native_tls::TlsConnector::new()
            .map_err(|error| format!("cannot set up TLS: {error}"))?;
        let tls = MakeTlsConnector::new(connector);
        let mut config = tokio_postgres::Config::new();
        config
            .host(&record.host)
            .port(u16::try_from(record.port).map_err(|_| "the port is out of range".to_string())?)
            .user(&record.username)
            .password(&record.password)
            .application_name("noverdbee")
            .ssl_mode(match record.ssl_mode.as_str() {
                "disable" => SslMode::Disable,
                "require" => SslMode::Require,
                _ => SslMode::Prefer,
            });
        if !record.database.is_empty() {
            config.dbname(&record.database);
        }
        let (client, connection) = config
            .connect(tls.clone())
            .await
            .map_err(|error| describe(&error))?;
        // The connection does the I/O; it ends when the client is dropped.
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(error = %error, "PostgreSQL connection closed");
            }
        });
        Ok(Self {
            cancel: client.cancel_token(),
            client: Arc::new(client),
            tls,
        })
    }

    /// Runs `sql` as one simple query. The server answers a failed query with
    /// the error alone, and undoes the statements before it unless they ran in
    /// a transaction the SQL opened itself, so an error is the only result.
    /// Each result carries the time the whole query took.
    // ponytail: every row crosses the wire before the limit applies; use a
    // cursor if results far beyond ROW_LIMIT become common.
    pub async fn run(&self, sql: String) -> Vec<StatementResult> {
        let started = Instant::now();
        let messages = self.client.simple_query(&sql).await;
        let elapsed = started.elapsed();
        let result = |outcome| StatementResult {
            sql: String::new(),
            outcome,
            elapsed,
        };
        let messages = match messages {
            Ok(messages) => messages,
            Err(error) => {
                return vec![result(Outcome::Error {
                    message: describe(&error),
                    position: position(&error, &sql),
                })]
            }
        };

        let mut results = Vec::new();
        let mut rows: Option<ResultSet> = None;
        for message in messages {
            match message {
                SimpleQueryMessage::RowDescription(columns) => {
                    rows = Some(ResultSet {
                        columns: columns
                            .iter()
                            .map(|column| Column {
                                name: column.name().to_string(),
                                type_name: String::new(),
                                numeric: false,
                            })
                            .collect(),
                        ..ResultSet::default()
                    });
                }
                SimpleQueryMessage::Row(row) => {
                    let Some(result) = rows.as_mut() else {
                        continue;
                    };
                    if result.rows.len() == ROW_LIMIT {
                        result.truncated = true;
                        continue;
                    }
                    let values = (0..row.len()).map(|index| row.get(index).map(str::to_string));
                    result.rows.push(values.collect());
                }
                SimpleQueryMessage::CommandComplete(count) => {
                    results.push(result(match rows.take() {
                        Some(mut result) => {
                            mark_numeric(&mut result);
                            Outcome::Rows(result)
                        }
                        None => Outcome::Affected(count),
                    }));
                }
                _ => {}
            }
        }
        results
    }

    /// The last result of `sql`, which must be rows.
    async fn rows(&self, sql: String) -> Result<ResultSet, String> {
        match self.run(sql).await.pop().map(|result| result.outcome) {
            Some(Outcome::Rows(result)) => Ok(result),
            Some(Outcome::Error { message, .. }) => Err(message),
            _ => Err("the server returned no rows".to_string()),
        }
    }

    pub async fn tables(&self) -> Result<Vec<TableInfo>, String> {
        let listed = self
            .rows(
                "SELECT c.table_schema, c.table_name, t.table_type, c.column_name, c.data_type, \
                        c.is_nullable, c.column_default, \
                        EXISTS (SELECT 1 FROM information_schema.table_constraints tc \
                                JOIN information_schema.key_column_usage k \
                                  ON k.constraint_schema = tc.constraint_schema \
                                 AND k.constraint_name = tc.constraint_name \
                                WHERE tc.constraint_type = 'PRIMARY KEY' \
                                  AND tc.table_schema = c.table_schema \
                                  AND tc.table_name = c.table_name \
                                  AND k.column_name = c.column_name) \
                 FROM information_schema.columns c \
                 JOIN information_schema.tables t \
                   ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
                 WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema') \
                 ORDER BY c.table_schema, c.table_name, c.ordinal_position"
                    .to_string(),
            )
            .await?;
        let mut tables: Vec<TableInfo> = Vec::new();
        for row in listed.rows {
            let mut cells = row.into_iter();
            let mut text = || cells.next().flatten().unwrap_or_default();
            let table = TableRef {
                schema: Some(text()),
                name: text(),
            };
            let kind = if text() == "VIEW" {
                ObjectKind::View
            } else {
                ObjectKind::Table
            };
            let (name, type_name, nullable) = (text(), text(), text() == "YES");
            let default = cells.next().flatten();
            let column = ColumnInfo {
                name,
                type_name,
                nullable,
                default,
                primary_key: cells.next().flatten().as_deref() == Some("t"),
            };
            match tables.last_mut() {
                Some(last) if last.table == table => last.columns.push(column),
                _ => tables.push(TableInfo {
                    table,
                    kind,
                    columns: vec![column],
                }),
            }
        }
        Ok(tables)
    }

    pub async fn version(&self) -> Result<String, String> {
        let result = self.rows("SELECT version()".to_string()).await?;
        let version = result.rows.into_iter().flatten().flatten().next();
        // "PostgreSQL 17.2 on aarch64-..." keeps its first two words.
        let version = version.unwrap_or_default();
        let short: Vec<&str> = version.split_whitespace().take(2).collect();
        Ok(short.join(" "))
    }

    pub fn cancel(&self) {
        let (cancel, tls) = (self.cancel.clone(), self.tls.clone());
        tokio::spawn(async move {
            if let Err(error) = cancel.cancel_query(tls).await {
                tracing::warn!(error = %error, "Failed to cancel the query");
            }
        });
    }
}

/// Simple queries carry no types, so a column counts as numeric when every
/// value in it reads as a number.
fn mark_numeric(result: &mut ResultSet) {
    for (index, column) in result.columns.iter_mut().enumerate() {
        let mut values = result.rows.iter().filter_map(|row| row[index].as_deref());
        column.numeric = values.clone().next().is_some()
            && values.all(|value| value.parse::<f64>().is_ok_and(f64::is_finite));
    }
}

/// The server's own message when it sent one; the driver's otherwise.
fn describe(error: &tokio_postgres::Error) -> String {
    match error.as_db_error() {
        Some(db_error) => db_error.message().to_string(),
        None => error.to_string(),
    }
}

/// Where in `sql` the server says the error is, as a byte offset. The server
/// counts characters from 1.
fn position(error: &tokio_postgres::Error, sql: &str) -> Option<usize> {
    match error.as_db_error()?.position()? {
        ErrorPosition::Original(character) => {
            let index = usize::try_from(*character).ok()?.checked_sub(1)?;
            sql.char_indices().nth(index).map(|(offset, _)| offset)
        }
        ErrorPosition::Internal { .. } => None,
    }
}

pub fn structure_queries(table: &TableRef) -> Vec<(&'static str, String)> {
    let schema = quote_literal(&Some(
        table.schema.clone().unwrap_or_else(|| "public".into()),
    ));
    let name = quote_literal(&Some(table.name.clone()));
    let regclass = quote_literal(&Some(table.quoted()));
    let of_table = format!("table_schema = {schema} AND table_name = {name}");
    vec![
        (
            "Columns",
            format!(
                "SELECT column_name AS name, data_type AS type, is_nullable AS nullable, \
                        column_default AS \"default\" \
                 FROM information_schema.columns WHERE {of_table} ORDER BY ordinal_position"
            ),
        ),
        (
            "Indexes",
            format!(
                "SELECT indexname AS name, indexdef AS definition FROM pg_indexes \
                 WHERE schemaname = {schema} AND tablename = {name} ORDER BY indexname"
            ),
        ),
        (
            "Foreign keys",
            format!(
                "SELECT conname AS name, pg_get_constraintdef(oid) AS definition \
                 FROM pg_constraint WHERE contype = 'f' AND conrelid = {regclass}::regclass \
                 ORDER BY conname"
            ),
        ),
        (
            // PostgreSQL keeps no CREATE TABLE text, so this rebuilds one.
            "Definition",
            format!(
                "SELECT coalesce( \
                    (SELECT view_definition::text FROM information_schema.views WHERE {of_table}), \
                    (SELECT '-- rebuilt from the catalog; keys and indexes are listed separately' \
                         || chr(10) || 'CREATE TABLE ' || {regclass} || ' (' || chr(10) \
                         || string_agg('  ' || quote_ident(column_name) || ' ' || data_type \
                                || CASE WHEN is_nullable = 'NO' THEN ' NOT NULL' ELSE '' END \
                                || coalesce(' DEFAULT ' || column_default, ''), \
                                ',' || chr(10) ORDER BY ordinal_position) \
                         || chr(10) || ')' \
                     FROM information_schema.columns WHERE {of_table}))"
            ),
        ),
    ]
}
