//! An MCP server on 127.0.0.1 that lets AI clients (Claude Code and others)
//! read and change the user's databases. Each saved connection says which
//! tools it allows; a connection that allows none stays out of MCP entirely.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use shared::{DbConnectionRecord, McpAccess, Setting};
use tokio::sync::oneshot;
use tracing::{info, warn};
use uuid::Uuid;

use crate::app::AppHandle;
use crate::db::sql::{quote_ident, quote_literal, TableRef};
use crate::db::{Session, DRIVER_POSTGRES};

/// noverterm listens on 47321 by default; this one stays well clear of it.
pub const DEFAULT_PORT: u16 = 47450;
const PORT_SETTING: &str = "noverdbee.mcp.port";
const TOKEN_SETTING: &str = "noverdbee.mcp.token";
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_ROWS: u64 = 200;
const MAX_ROWS: u64 = 2000;
const MAX_INSERT_ROWS: usize = 1000;
const ACTIVITY_LIMIT: usize = 20;

#[derive(Default)]
pub struct McpState {
    /// The bearer token clients must send; empty until the server started.
    token: Mutex<String>,
    /// The port the user chose; 0 until it has been read.
    port: Mutex<u16>,
    server: Mutex<Option<RunningServer>>,
    /// Why the server is not running.
    error: Mutex<Option<String>>,
    /// Recent tool calls, newest first.
    activity: Mutex<VecDeque<McpActivity>>,
}

struct RunningServer {
    port: u16,
    shutdown: oneshot::Sender<()>,
}

/// One tool call that touched a connection.
#[derive(Debug, Clone)]
pub struct McpActivity {
    pub connection: String,
    pub tool: String,
    /// The SQL, table, or condition that says what it did.
    pub detail: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpStatus {
    pub port: u16,
    /// Listening, with `url` and `token` ready for clients.
    pub running: Option<(String, String)>,
    pub error: Option<String>,
}

pub fn status(app: &AppHandle) -> McpStatus {
    let state = app.state::<McpState>();
    let token = state.token.lock().unwrap().clone();
    let running = state
        .server
        .lock()
        .unwrap()
        .as_ref()
        .map(|server| server.port);
    let port = *state.port.lock().unwrap();
    let error = state.error.lock().unwrap().clone();
    McpStatus {
        port: if port == 0 { DEFAULT_PORT } else { port },
        running: running
            .filter(|_| !token.is_empty())
            .map(|port| (format!("http://127.0.0.1:{port}/mcp"), token)),
        error,
    }
}

pub fn activity(app: &AppHandle) -> Vec<McpActivity> {
    app.state::<McpState>()
        .activity
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect()
}

/// Replaces the token; clients set up with the old one stop working.
pub async fn regenerate_token(app: AppHandle) -> Result<(), String> {
    let token = new_token();
    save_setting(&app, TOKEN_SETTING, token.clone()).await?;
    *app.state::<McpState>().token.lock().unwrap() = token;
    Ok(())
}

/// Saves `port` and moves the server there.
pub async fn set_port(app: AppHandle, port: u16) -> Result<(), String> {
    save_setting(&app, PORT_SETTING, port.to_string()).await?;
    *app.state::<McpState>().port.lock().unwrap() = port;
    restart(&app).await
}

fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

async fn save_setting(app: &AppHandle, key: &str, value: String) -> Result<(), String> {
    let setting = Setting {
        key: key.to_string(),
        value,
    };
    crate::store::settings::set_setting(setting, app.state())
        .await
        .map(|_| ())
}

async fn load_setting(app: &AppHandle, key: &str) -> Result<Option<String>, String> {
    let saved = crate::store::settings::get_setting(key.to_string(), app.state()).await?;
    Ok(saved.map(|setting| setting.value))
}

/// Starts the server at launch. It listens on loopback only, every request
/// needs the bearer token, and no connection is shared until the user ticks
/// a tool on it, so it always runs.
pub async fn start(app: AppHandle) {
    let loaded = async {
        let token = match load_setting(&app, TOKEN_SETTING).await? {
            Some(token) => token,
            None => {
                let token = new_token();
                save_setting(&app, TOKEN_SETTING, token.clone()).await?;
                token
            }
        };
        let port = load_setting(&app, PORT_SETTING)
            .await?
            .and_then(|port| port.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Ok::<_, String>((token, port))
    };
    let result = match loaded.await {
        Ok((token, port)) => {
            let state = app.state::<McpState>();
            *state.token.lock().unwrap() = token;
            *state.port.lock().unwrap() = port;
            restart(&app).await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        warn!(error, "MCP server did not start");
    }
}

/// Stops any running server, then listens on the chosen port. The error, if
/// any, is also kept for the MCP page.
async fn restart(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<McpState>();
    if let Some(running) = state.server.lock().unwrap().take() {
        let _ = running.shutdown.send(());
        info!(port = running.port, "MCP server stopped");
    }
    let port = *state.port.lock().unwrap();
    let result = listen(app, port).await;
    *state.error.lock().unwrap() = result.as_ref().err().cloned();
    let server = result?;
    *state.server.lock().unwrap() = Some(server);
    Ok(())
}

async fn listen(app: &AppHandle, port: u16) -> Result<RunningServer, String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| format!("Could not listen on port {port}: {error}"))?;
    let router = Router::new()
        .route("/mcp", post(handle).get(not_allowed).delete(not_allowed))
        .with_state(Arc::new(app.clone()));
    let (shutdown, stopped) = oneshot::channel::<()>();
    crate::app::runtime().spawn(async move {
        let serve = axum::serve(listener, router).with_graceful_shutdown(async {
            let _ = stopped.await;
        });
        if let Err(error) = serve.await {
            warn!(%error, "MCP server exited");
        }
    });
    info!(port, "MCP server listening");
    Ok(RunningServer { port, shutdown })
}

async fn not_allowed() -> StatusCode {
    // No server-initiated stream: clients fall back to plain POST replies.
    StatusCode::METHOD_NOT_ALLOWED
}

async fn handle(State(app): State<Arc<AppHandle>>, headers: HeaderMap, body: Bytes) -> Response {
    // A web page in the browser can reach loopback; refuse anything it sends.
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let local = ["http://127.0.0.1", "http://localhost"]
            .iter()
            .any(|allowed| origin == *allowed || origin.starts_with(&format!("{allowed}:")));
        if !local {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    let token = app.state::<McpState>().token.lock().unwrap().clone();
    if token.is_empty() || !constant_time_eq(presented.as_bytes(), token.as_bytes()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_error(Value::Null, -32700, "Parse error")),
        )
            .into_response();
    };
    // Notifications (no id) need no reply.
    let Some(id) = message.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let reply = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let requested = params["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|version| **version == requested)
                .unwrap_or(&PROTOCOL_VERSIONS[0]);
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "noverdbee", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Query and change the user's databases. Call list_connections first: \
                        it shows each connection and the tools it allows. Use list_tables and describe_table \
                        before writing SQL. Prefer select; change data only when the user asked for it.",
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": tools() })),
        "tools/call" => {
            let (text, is_error) = match call_tool(&app, &params).await {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            rpc_result(
                id,
                json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
            )
        }
        method => rpc_error(id, -32601, &format!("Method not found: {method}")),
    };
    Json(reply).into_response()
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

fn tools() -> Value {
    let connection =
        json!({ "type": "string", "description": "Connection name from list_connections." });
    let table = json!({ "type": "string", "description": "Table name; `schema.table` for PostgreSQL outside public." });
    let condition = json!({ "type": "string", "description": "SQL condition after WHERE, e.g. `id = 7`. Required; write `1=1` to mean every row." });
    json!([
        tool(
            "list_connections",
            "Lists the connections shared with MCP and the tools each allows.",
            json!({}),
            &[],
            json!({ "readOnlyHint": true })
        ),
        tool(
            "list_tables",
            "Lists the tables and views of a connection.",
            json!({ "connection": connection }),
            &["connection"],
            json!({ "readOnlyHint": true })
        ),
        tool(
            "describe_table",
            "Lists a table's columns: name, type, nullable, default, primary key.",
            json!({ "connection": connection, "table": table }),
            &["connection", "table"],
            json!({ "readOnlyHint": true })
        ),
        tool(
            "select",
            "Runs one read-only query (SELECT, WITH or VALUES) and returns rows as JSON objects.",
            json!({
                "connection": connection,
                "sql": { "type": "string", "description": "One query." },
                "limit": { "type": "integer", "description": "Most rows to return (default 200, max 2000)." },
            }),
            &["connection", "sql"],
            json!({ "readOnlyHint": true })
        ),
        tool(
            "insert",
            "Inserts rows into a table and returns how many were inserted.",
            json!({
                "connection": connection,
                "table": table,
                "rows": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "Rows as {column: value} objects, all with the same columns. Columns left out take their defaults.",
                },
            }),
            &["connection", "table", "rows"],
            json!({ "destructiveHint": false })
        ),
        tool(
            "update",
            "Updates the rows matching a condition and returns how many changed.",
            json!({
                "connection": connection,
                "table": table,
                "set": { "type": "object", "description": "New values as {column: value}." },
                "where": condition,
            }),
            &["connection", "table", "set", "where"],
            json!({ "destructiveHint": true })
        ),
        tool(
            "delete",
            "Deletes the rows matching a condition and returns how many were deleted.",
            json!({ "connection": connection, "table": table, "where": condition }),
            &["connection", "table", "where"],
            json!({ "destructiveHint": true })
        ),
    ])
}

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    annotations: Value,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": annotations,
    })
}

async fn call_tool(app: &AppHandle, params: &Value) -> Result<String, String> {
    let name = params["name"].as_str().unwrap_or_default();
    let args = &params["arguments"];
    if name == "list_connections" {
        return list_connections(app).await;
    }
    let result = dispatch(app, name, args).await;
    let detail = ["sql", "where", "table"]
        .into_iter()
        .find_map(|key| args[key].as_str())
        .unwrap_or_default();
    let entry = McpActivity {
        connection: args["connection"].as_str().unwrap_or_default().to_string(),
        tool: name.to_string(),
        detail: detail.to_string(),
        error: result.as_ref().err().cloned(),
    };
    let state = app.state::<McpState>();
    let mut activity = state.activity.lock().unwrap();
    activity.push_front(entry);
    activity.truncate(ACTIVITY_LIMIT);
    result
}

async fn dispatch(app: &AppHandle, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "list_tables" => {
            let record = connection(app, args, "", |_| true).await?;
            let tables = Session::open(&record).await?.tables().await?;
            Ok(tables
                .iter()
                .map(|info| {
                    let kind = match info.kind {
                        crate::db::ObjectKind::Table => "table",
                        crate::db::ObjectKind::View => "view",
                    };
                    format!("{} ({kind})", info.table.label())
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "describe_table" => {
            let record = connection(app, args, "", |_| true).await?;
            let table = table_ref(&record, required(args, "table")?);
            let tables = Session::open(&record).await?.tables().await?;
            let info = tables
                .iter()
                .find(|info| info.table == table)
                .ok_or_else(|| format!("No table {} in {}.", table.label(), record.name))?;
            let columns: Vec<Value> = info
                .columns
                .iter()
                .map(|column| {
                    json!({
                        "name": column.name,
                        "type": column.type_name,
                        "nullable": column.nullable,
                        "default": column.default,
                        "primary_key": column.primary_key,
                    })
                })
                .collect();
            Ok(Value::from(columns).to_string())
        }
        "select" => {
            let record = connection(app, args, "select", |access| access.select).await?;
            let sql = required(args, "sql")?.to_string();
            let limit = args["limit"]
                .as_u64()
                .unwrap_or(DEFAULT_ROWS)
                .clamp(1, MAX_ROWS);
            // Its own read-only session: SQLite opens the file read-only, and
            // PostgreSQL runs the query in a read-only transaction.
            let session = Session::open(&DbConnectionRecord {
                read_only: true,
                ..record
            })
            .await?;
            let (rows, truncated) = session.select_json(sql, limit as usize).await?;
            let mut text = format!("[{}]", rows.join(","));
            if truncated {
                text.push_str(&format!(
                    "\n(stopped at {limit} rows; raise limit or narrow the query)"
                ));
            }
            Ok(text)
        }
        "insert" => {
            let record = connection(app, args, "insert", |access| access.insert).await?;
            let table = table_ref(&record, required(args, "table")?);
            let rows: Vec<&Map<String, Value>> = args["rows"]
                .as_array()
                .ok_or("rows must be an array of objects")?
                .iter()
                .map(|row| row.as_object().ok_or("rows must be an array of objects"))
                .collect::<Result<_, _>>()?;
            write(&record, insert_sql(&table, &rows)?, "inserted").await
        }
        "update" => {
            let record = connection(app, args, "update", |access| access.update).await?;
            let table = table_ref(&record, required(args, "table")?);
            let set = args["set"].as_object().ok_or("set must be an object")?;
            write(
                &record,
                update_sql(&table, set, required(args, "where")?)?,
                "updated",
            )
            .await
        }
        "delete" => {
            let record = connection(app, args, "delete", |access| access.delete).await?;
            let table = table_ref(&record, required(args, "table")?);
            write(
                &record,
                delete_sql(&table, required(args, "where")?)?,
                "deleted",
            )
            .await
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

async fn write(record: &DbConnectionRecord, sql: String, verb: &str) -> Result<String, String> {
    let count = Session::open(record).await?.execute_one(sql).await?;
    Ok(format!("{count} rows {verb}."))
}

fn required<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args[name]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

async fn list_connections(app: &AppHandle) -> Result<String, String> {
    let connections = crate::store::connections::connection_list(app.state()).await?;
    let shared: Vec<Value> = connections
        .iter()
        .filter(|record| record.mcp.any())
        .map(|record| {
            json!({
                "name": record.name,
                "driver": record.driver,
                "environment": record.environment.as_str(),
                "tools": tool_names(record.mcp),
            })
        })
        .collect();
    if shared.is_empty() {
        return Ok("No connection is shared with MCP. The user can allow tools on a connection in noverdbee.".to_string());
    }
    Ok(Value::from(shared).to_string())
}

fn tool_names(access: McpAccess) -> Vec<&'static str> {
    let mut names = vec!["list_tables", "describe_table"];
    for (allowed, name) in [
        (access.select, "select"),
        (access.insert, "insert"),
        (access.update, "update"),
        (access.delete, "delete"),
    ] {
        if allowed {
            names.push(name);
        }
    }
    names
}

/// The connection named in `args`, once it allows `tool`.
async fn connection(
    app: &AppHandle,
    args: &Value,
    tool: &str,
    allowed: fn(McpAccess) -> bool,
) -> Result<DbConnectionRecord, String> {
    let name = required(args, "connection")?;
    let record = crate::store::connections::connection_list(app.state())
        .await?
        .into_iter()
        .find(|record| record.name == name && record.mcp.any())
        .ok_or_else(|| {
            format!("No connection named {name} is shared with MCP. Call list_connections.")
        })?;
    if !allowed(record.mcp) {
        return Err(format!(
            "{name} does not allow {tool} over MCP. The user can allow it in the connection's settings in noverdbee."
        ));
    }
    Ok(record)
}

fn table_ref(record: &DbConnectionRecord, name: &str) -> TableRef {
    if record.driver != DRIVER_POSTGRES {
        return TableRef {
            schema: None,
            name: name.to_string(),
        };
    }
    let (schema, name) = name.split_once('.').unwrap_or(("public", name));
    TableRef {
        schema: Some(schema.to_string()),
        name: name.to_string(),
    }
}

/// A JSON value as an SQL literal.
fn literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(true) => "TRUE".to_string(),
        Value::Bool(false) => "FALSE".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => quote_literal(&Some(text.clone())),
        // Objects and arrays go in as JSON text, for json columns.
        other => quote_literal(&Some(other.to_string())),
    }
}

fn insert_sql(table: &TableRef, rows: &[&Map<String, Value>]) -> Result<String, String> {
    let first = rows.first().ok_or("rows is empty")?;
    if first.is_empty() {
        return Err("each row needs at least one column".to_string());
    }
    if rows.len() > MAX_INSERT_ROWS {
        return Err(format!("insert at most {MAX_INSERT_ROWS} rows at a time"));
    }
    if rows.iter().any(|row| !row.keys().eq(first.keys())) {
        return Err("every row must have the same columns".to_string());
    }
    let columns: Vec<String> = first.keys().map(|column| quote_ident(column)).collect();
    let values: Vec<String> = rows
        .iter()
        .map(|row| {
            let literals: Vec<String> = row.values().map(literal).collect();
            format!("({})", literals.join(", "))
        })
        .collect();
    Ok(format!(
        "INSERT INTO {} ({}) VALUES {}",
        table.quoted(),
        columns.join(", "),
        values.join(", ")
    ))
}

fn update_sql(
    table: &TableRef,
    set: &Map<String, Value>,
    condition: &str,
) -> Result<String, String> {
    if set.is_empty() {
        return Err("set needs at least one column".to_string());
    }
    let changes: Vec<String> = set
        .iter()
        .map(|(column, value)| format!("{} = {}", quote_ident(column), literal(value)))
        .collect();
    Ok(format!(
        "UPDATE {} SET {} WHERE {}",
        table.quoted(),
        changes.join(", "),
        condition.trim()
    ))
}

fn delete_sql(table: &TableRef, condition: &str) -> Result<String, String> {
    Ok(format!(
        "DELETE FROM {} WHERE {}",
        table.quoted(),
        condition.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("an object")
    }

    #[test]
    fn builds_writes_from_json() {
        let table = TableRef {
            schema: Some("public".into()),
            name: "users".into(),
        };
        let first = object(json!({ "name": "O'Brien", "age": 30, "admin": true, "tags": ["a"] }));
        let second = object(json!({ "name": "bo", "age": null, "admin": false, "tags": [] }));
        assert_eq!(
            insert_sql(&table, &[&first, &second]).unwrap(),
            r#"INSERT INTO "public"."users" ("name", "age", "admin", "tags") VALUES ('O''Brien', 30, TRUE, '["a"]'), ('bo', NULL, FALSE, '[]')"#
        );
        let other = object(json!({ "name": "cy" }));
        assert!(insert_sql(&table, &[&first, &other]).is_err());
        assert!(insert_sql(&table, &[]).is_err());

        assert_eq!(
            update_sql(&table, &object(json!({ "name": "x" })), " id = 7 ").unwrap(),
            r#"UPDATE "public"."users" SET "name" = 'x' WHERE id = 7"#
        );
        assert!(update_sql(&table, &Map::new(), "id = 7").is_err());
        assert_eq!(
            delete_sql(&table, "id = 7").unwrap(),
            r#"DELETE FROM "public"."users" WHERE id = 7"#
        );
    }

    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        listener.local_addr().expect("address").port()
    }

    #[tokio::test]
    async fn changing_the_port_moves_the_server() {
        let (app, _directory) = crate::store::tests::test_app();
        app.manage(McpState::default());
        let first = free_port();
        save_setting(&app, PORT_SETTING, first.to_string())
            .await
            .expect("save port");
        start(app.clone()).await;
        let (url, _) = status(&app).running.expect("running");
        assert_eq!(url, format!("http://127.0.0.1:{first}/mcp"));

        let second = free_port();
        set_port(app.clone(), second).await.expect("move");
        assert_eq!(status(&app).port, second);
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", second))
            .await
            .is_ok());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", first))
            .await
            .is_err());

        // A port in use leaves the server stopped and says why.
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let busy = taken.local_addr().expect("address").port();
        assert!(set_port(app.clone(), busy).await.is_err());
        assert!(status(&app).running.is_none());
        assert!(status(&app).error.is_some());
    }

    #[test]
    fn postgres_tables_default_to_public() {
        let postgres = DbConnectionRecord {
            driver: DRIVER_POSTGRES.into(),
            ..Default::default()
        };
        assert_eq!(
            table_ref(&postgres, "users").schema.as_deref(),
            Some("public")
        );
        assert_eq!(
            table_ref(&postgres, "audit.log").schema.as_deref(),
            Some("audit")
        );
        let sqlite = DbConnectionRecord::default();
        assert_eq!(table_ref(&sqlite, "a.b").name, "a.b");
    }
}
