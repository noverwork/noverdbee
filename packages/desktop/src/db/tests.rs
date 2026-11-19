use shared::DbConnectionRecord;
use tempfile::TempDir;

use super::sql::{self, Direction};
use super::{
    ObjectKind, Outcome, ResultSet, Session, SessionManager, TableRef, DRIVER_POSTGRES,
    DRIVER_SQLITE,
};

fn record(directory: &TempDir, read_only: bool) -> DbConnectionRecord {
    DbConnectionRecord {
        id: "c1".to_string(),
        name: "scratch".to_string(),
        driver: DRIVER_SQLITE.to_string(),
        path: directory
            .path()
            .join("scratch.db")
            .to_string_lossy()
            .into_owned(),
        read_only,
        ..DbConnectionRecord::default()
    }
}

async fn rows(session: &Session, sql: &str) -> ResultSet {
    let last = session.run(sql.to_string()).await.pop();
    match last.map(|result| result.outcome) {
        Some(Outcome::Rows(result)) => result,
        other => panic!("expected rows from {sql}, got {other:?}"),
    }
}

fn cells(values: &[Option<&str>]) -> Vec<Option<String>> {
    values
        .iter()
        .map(|value| value.map(str::to_string))
        .collect()
}

#[tokio::test]
async fn sqlite_runs_statements_in_order_and_reports_each_outcome() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");

    let results = session
        .run(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT DEFAULT 'x');
             INSERT INTO users (name, note) VALUES ('ann', NULL), ('O''Brien', 'hi');
             SELECT id, name, note, id * 2 FROM users ORDER BY id;"
                .to_string(),
        )
        .await;
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].outcome, Outcome::Affected(0));
    assert_eq!(results[1].outcome, Outcome::Affected(2));
    let Outcome::Rows(result) = &results[2].outcome else {
        panic!("expected rows, got {:?}", results[2].outcome);
    };
    let numeric: Vec<bool> = result.columns.iter().map(|column| column.numeric).collect();
    assert_eq!(numeric, [true, false, false, true]);
    assert_eq!(
        result.rows,
        [
            cells(&[Some("1"), Some("ann"), None, Some("2")]),
            cells(&[Some("2"), Some("O'Brien"), Some("hi"), Some("4")]),
        ]
    );

    let tables = session.tables().await.expect("tables");
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].kind, ObjectKind::Table);
    let columns = &tables[0].columns;
    assert_eq!(columns.len(), 3);
    assert!(columns[0].primary_key && !columns[1].primary_key);
    assert!(!columns[1].nullable && columns[2].nullable);
    assert_eq!(columns[2].default.as_deref(), Some("'x'"));
    let version = session.version().await.expect("version");
    assert!(version.starts_with("SQLite 3."), "{version}");
}

#[tokio::test]
async fn sqlite_stops_at_the_first_error_and_points_at_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");

    let sql = "CREATE TABLE t (x); SELECT * FROM t WHERE; SELECT 1";
    let results = session.run(sql.to_string()).await;
    assert_eq!(
        results.len(),
        2,
        "the statement after the error must not run"
    );
    let Outcome::Error { message, position } = &results[1].outcome else {
        panic!("expected an error, got {:?}", results[1].outcome);
    };
    assert!(message.contains("syntax error"), "{message}");
    assert_eq!(position.map(|at| &sql[at..at + 1]), Some(";"));
}

#[tokio::test]
async fn generated_statements_round_trip_awkward_values() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");
    session
        .run("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)".to_string())
        .await;
    let table = TableRef {
        schema: None,
        name: "notes".to_string(),
    };
    let pair = |column: &str, value: Option<&str>| (column.to_string(), value.map(str::to_string));
    let awkward = "it's \"quoted\"; DROP TABLE notes; -- \\ 🙂";

    for statement in [
        sql::insert(&table, &[pair("body", Some("first"))]),
        sql::insert(&table, &[pair("body", None)]),
        sql::update(
            &table,
            &[pair("id", Some("1"))],
            &[pair("body", Some(awkward))],
        ),
        sql::delete(&table, &[pair("id", Some("2"))]),
    ] {
        let results = session.run(statement.clone()).await;
        assert_eq!(results[0].outcome, Outcome::Affected(1), "{statement}");
    }

    let page = sql::page_query(&table, "", Some(("id", Direction::Ascending)), 200, 0);
    assert_eq!(
        rows(&session, &page).await.rows,
        [cells(&[Some("1"), Some(awkward)])]
    );
}

#[tokio::test]
async fn a_read_only_session_rejects_writes_and_a_non_database_is_rejected() {
    let directory = tempfile::tempdir().expect("temp dir");
    Session::open(&record(&directory, false))
        .await
        .expect("create")
        .run("CREATE TABLE t (x)".to_string())
        .await;

    let manager = SessionManager::default();
    let read_only = record(&directory, true);
    let session = manager.connect(&read_only).await.expect("open read-only");
    let results = session.run("INSERT INTO t VALUES (1)".to_string()).await;
    assert!(matches!(results[0].outcome, Outcome::Error { .. }));
    assert!(manager.get(&read_only.id).is_some());
    manager.disconnect(&read_only.id);
    assert!(manager.get(&read_only.id).is_none());

    let junk = directory.path().join("junk.db");
    std::fs::write(&junk, "not a database, just some text").expect("write junk");
    let mut bad = record(&directory, false);
    bad.path = junk.to_string_lossy().into_owned();
    assert!(Session::open(&bad).await.is_err());
}

#[tokio::test]
async fn sqlite_check_parses_without_running() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");
    session.run("CREATE TABLE t (x)".to_string()).await;

    assert_eq!(
        session
            .check("SELECT x FROM t; DELETE FROM t".to_string())
            .await,
        None
    );
    let sql = "INSERT INTO t VALUES (1); SELECT * FROM t WHERE; SELECT 1";
    let (message, at) = session.check(sql.to_string()).await.expect("an error");
    assert!(message.contains("syntax error"), "{message}");
    assert_eq!(&sql[at..at + 1], ";");
    let (message, _) = session
        .check("SELECT nope FROM t".to_string())
        .await
        .expect("an error");
    assert!(message.contains("nope"), "{message}");
    assert_eq!(
        rows(&session, "SELECT count(*) FROM t").await.rows,
        [cells(&[Some("0")])]
    );
}

/// A connection record for the server at `url`.
fn pg_record(url: &str) -> DbConnectionRecord {
    let config: tokio_postgres::Config = url.parse().expect("a PostgreSQL URL");
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => panic!("the URL needs a TCP host"),
    };
    DbConnectionRecord {
        id: "pg".to_string(),
        name: "test".to_string(),
        driver: DRIVER_POSTGRES.to_string(),
        host,
        port: i32::from(config.get_ports().first().copied().unwrap_or(5432)),
        database: config.get_dbname().unwrap_or("postgres").to_string(),
        username: config.get_user().unwrap_or("postgres").to_string(),
        password: String::from_utf8_lossy(config.get_password().unwrap_or_default()).into_owned(),
        ssl_mode: "prefer".to_string(),
        ..DbConnectionRecord::default()
    }
}

/// Needs a server: set `NOVERDBEE_TEST_PG_URL`, for example
/// `postgres://postgres:postgres@localhost:5432/postgres`. Skipped otherwise.
#[tokio::test]
async fn postgres_runs_lists_edits_and_points_at_errors() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let record = pg_record(&url);
    let session = Session::open(&record).await.expect("connect");
    let version = session.version().await.expect("version");
    assert!(version.starts_with("PostgreSQL "), "{version}");

    let schema = format!("noverdbee_test_{}", std::process::id());
    let results = session
        .run(format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE;
             CREATE SCHEMA {schema};
             CREATE TABLE {schema}.notes (id serial PRIMARY KEY, body text, score numeric NOT NULL DEFAULT 0);
             INSERT INTO {schema}.notes (body, score) VALUES ('first', 1.5), (NULL, 20);
             SELECT id, body, score FROM {schema}.notes ORDER BY id;"
        ))
        .await;
    assert_eq!(results.len(), 5, "{results:?}");
    assert_eq!(results[3].outcome, Outcome::Affected(2));
    let Outcome::Rows(result) = &results[4].outcome else {
        panic!("expected rows, got {:?}", results[4].outcome);
    };
    let numeric: Vec<bool> = result.columns.iter().map(|column| column.numeric).collect();
    assert_eq!(numeric, [true, false, true]);
    assert_eq!(
        result.rows,
        [
            cells(&[Some("1"), Some("first"), Some("1.5")]),
            cells(&[Some("2"), None, Some("20")]),
        ]
    );

    let table = TableRef {
        schema: Some(schema.clone()),
        name: "notes".to_string(),
    };
    let tables = session.tables().await.expect("tables");
    let info = tables
        .iter()
        .find(|info| info.table == table)
        .expect("the new table is listed");
    assert_eq!(info.kind, ObjectKind::Table);
    let keys: Vec<bool> = info
        .columns
        .iter()
        .map(|column| column.primary_key)
        .collect();
    assert_eq!(keys, [true, false, false]);
    assert!(info.columns[1].nullable && !info.columns[2].nullable);

    let pair = |column: &str, value: Option<&str>| (column.to_string(), value.map(str::to_string));
    let awkward = "it's \"quoted\"; DROP TABLE notes; -- \\ 🙂";
    for statement in [
        sql::insert(&table, &[pair("body", Some("third"))]),
        sql::update(
            &table,
            &[pair("id", Some("1"))],
            &[pair("body", Some(awkward)), pair("score", Some("7"))],
        ),
        sql::delete(&table, &[pair("id", Some("2"))]),
    ] {
        let results = session.run(statement.clone()).await;
        assert_eq!(results[0].outcome, Outcome::Affected(1), "{statement}");
    }
    let page = sql::page_query(
        &table,
        "score > 1",
        Some(("id", Direction::Ascending)),
        200,
        0,
    );
    assert_eq!(
        rows(&session, &page).await.rows,
        [cells(&[Some("1"), Some(awkward), Some("7")])]
    );

    for (title, query) in session.structure_queries(&table) {
        let outcome = session.run(query).await.pop().map(|result| result.outcome);
        assert!(
            matches!(outcome, Some(Outcome::Rows(_))),
            "{title}: {outcome:?}"
        );
    }

    let bad = format!("SELECT 1; SELECT * FROM {schema}.notes WHERE;");
    let results = session.run(bad.clone()).await;
    let Some(Outcome::Error { message, position }) = results.last().map(|last| &last.outcome)
    else {
        panic!("expected an error, got {results:?}");
    };
    assert!(message.contains("syntax error"), "{message}");
    assert_eq!(position.map(|at| &bad[at..]), Some(";"));

    // The check parses on its own connection, so a bad statement leaves the
    // transaction the user opened alone, and nothing it checks runs.
    session
        .run(format!(
            "BEGIN; INSERT INTO {schema}.notes (body) VALUES ('kept')"
        ))
        .await;
    assert_eq!(
        session
            .check(format!("SELECT 1; DELETE FROM {schema}.notes"))
            .await,
        None
    );
    let (message, at) = session.check(bad.clone()).await.expect("an error");
    assert!(message.contains("syntax error"), "{message}");
    assert_eq!(&bad[at..], ";");
    let missing = format!("SELECT 1;\nSELECT * FROM {schema}.missing");
    let (message, at) = session.check(missing.clone()).await.expect("an error");
    assert!(message.contains("missing"), "{message}");
    assert!(
        missing[at..].starts_with(&format!("{schema}.missing")),
        "{at}"
    );
    let outcome = session
        .run("COMMIT".to_string())
        .await
        .pop()
        .map(|result| result.outcome);
    assert_eq!(outcome, Some(Outcome::Affected(0)));
    let count = format!("SELECT count(*) FROM {schema}.notes WHERE body = 'kept'");
    assert_eq!(rows(&session, &count).await.rows, [cells(&[Some("1")])]);

    session.run(format!("DROP SCHEMA {schema} CASCADE")).await;
}

#[tokio::test]
async fn sqlite_mcp_calls_run_one_statement_and_select_cannot_write() {
    let directory = tempfile::tempdir().expect("temp dir");
    let writable = Session::open(&record(&directory, false))
        .await
        .expect("open");
    writable
        .run(
            "CREATE TABLE t (id INTEGER, name TEXT); INSERT INTO t VALUES (1, 'a'), (2, NULL);"
                .to_string(),
        )
        .await;

    assert_eq!(
        writable
            .execute_one("UPDATE t SET name = 'b' WHERE id = 2".to_string())
            .await,
        Ok(1)
    );
    // A condition cannot smuggle in a second statement.
    assert!(writable
        .execute_one("DELETE FROM t WHERE 1=1; DROP TABLE t".to_string())
        .await
        .is_err());

    let reader = Session::open(&record(&directory, true))
        .await
        .expect("open");
    assert_eq!(
        reader
            .select_json("SELECT id, name FROM t ORDER BY id".to_string(), 1)
            .await,
        Ok((vec![r#"{"id":1,"name":"a"}"#.to_string()], true))
    );
    assert!(reader
        .select_json("DELETE FROM t".to_string(), 10)
        .await
        .is_err());
    assert!(reader
        .select_json("SELECT 1; DELETE FROM t".to_string(), 10)
        .await
        .is_err());
    assert_eq!(
        rows(&writable, "SELECT count(*) FROM t").await.rows,
        vec![cells(&[Some("2")])]
    );
}

/// Needs a server, like the test above.
#[tokio::test]
async fn postgres_mcp_select_is_read_only_and_writes_run_one_statement() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let record = pg_record(&url);
    let table = format!("noverdbee_mcp_{}", std::process::id());
    let writer = Session::open(&record).await.expect("connect");
    writer
        .run(format!(
            "DROP TABLE IF EXISTS {table}; CREATE TABLE {table} (id int, name text, ok boolean);"
        ))
        .await;
    assert_eq!(
        writer
            .execute_one(format!(
                "INSERT INTO {table} (id, name, ok) VALUES (1, 'a', TRUE), (2, NULL, FALSE)"
            ))
            .await,
        Ok(2)
    );
    assert!(writer
        .execute_one(format!("DELETE FROM {table} WHERE 1=1; DROP TABLE {table}"))
        .await
        .is_err());

    // Each select gets a session of its own, as the MCP server does.
    let select = |sql: String| {
        let record = record.clone();
        async move {
            Session::open(&record)
                .await
                .expect("connect")
                .select_json(sql, 1)
                .await
        }
    };
    assert_eq!(
        select(format!("SELECT id, name, ok FROM {table} ORDER BY id;")).await,
        Ok((vec![r#"{"id":1,"name":"a","ok":true}"#.to_string()], true))
    );
    // A trailing comment cannot swallow the wrapper.
    assert!(select(format!("SELECT * FROM {table} -- note"))
        .await
        .is_ok());
    for sneaky in [
        format!("SELECT 1) t; DELETE FROM {table}; SELECT (1"),
        format!("WITH gone AS (DELETE FROM {table} RETURNING *) SELECT * FROM gone"),
        format!("SELECT 1 FROM {table}) t; COMMIT; DELETE FROM {table}; --"),
    ] {
        assert!(select(sneaky.clone()).await.is_err(), "{sneaky}");
    }
    assert_eq!(
        rows(&writer, &format!("SELECT count(*) FROM {table}"))
            .await
            .rows,
        vec![cells(&[Some("2")])]
    );
    writer.run(format!("DROP TABLE {table}")).await;
}

#[tokio::test]
async fn sqlite_paths_may_start_at_the_home_folder() {
    let home = dirs::home_dir().expect("a home folder");
    let directory = tempfile::tempdir_in(&home).expect("temp dir in home");
    let name = directory
        .path()
        .file_name()
        .expect("a name")
        .to_string_lossy();
    let record = DbConnectionRecord {
        driver: DRIVER_SQLITE.to_string(),
        path: format!("~/{name}/tilde.db"),
        ..DbConnectionRecord::default()
    };
    let session = Session::open(&record).await.expect("open ~ path");
    session.run("CREATE TABLE t (id INTEGER)".to_string()).await;
    assert!(directory.path().join("tilde.db").exists());
}

/// Needs a server, as above.
#[tokio::test]
async fn postgres_reopens_a_session_the_server_closed() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let record = DbConnectionRecord {
        id: "reconnect".to_string(),
        ..pg_record(&url)
    };
    let manager = super::SessionManager::default();
    let first = manager.connect(&record).await.expect("connect");
    let killer = Session::open(&record).await.expect("second connection");
    let pid = rows(&first, "SELECT pg_backend_pid()").await.rows[0][0].clone();
    rows(
        &killer,
        &format!("SELECT pg_terminate_backend({})", pid.expect("a pid")),
    )
    .await;
    for _ in 0..50 {
        if first.is_closed() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(first.is_closed(), "the terminated session notices");

    let second = manager.connect(&record).await.expect("reconnect");
    assert!(!second.is_closed());
    assert_eq!(rows(&second, "SELECT 1").await.rows, [cells(&[Some("1")])]);
}

#[tokio::test]
async fn sqlite_reports_an_open_transaction() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");
    assert!(!session.in_transaction().await);
    session.run("BEGIN".to_string()).await;
    assert!(session.in_transaction().await);
    session.run("ROLLBACK".to_string()).await;
    assert!(!session.in_transaction().await);
}

/// Needs a server, as above.
#[tokio::test]
async fn postgres_reports_an_open_and_a_failed_transaction() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let session = Session::open(&pg_record(&url)).await.expect("connect");
    assert!(!session.in_transaction().await);
    session.run("BEGIN".to_string()).await;
    assert!(session.in_transaction().await);
    session.run("SELECT 1 / 0".to_string()).await;
    assert!(
        session.in_transaction().await,
        "a failed transaction is still open"
    );
    session.run("ROLLBACK".to_string()).await;
    assert!(!session.in_transaction().await);
}

/// What a table tab's save does inside a transaction the user opened: a
/// failed save rolls back to its savepoint and the user's work survives.
async fn a_failed_save_keeps_the_open_transaction(session: &Session, table: &str) {
    session
        .run(format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY)"))
        .await;
    session
        .run(format!("BEGIN; INSERT INTO {table} VALUES (1)"))
        .await;
    session
        .run(format!(
            "SAVEPOINT noverdbee_save; INSERT INTO {table} VALUES (1); RELEASE SAVEPOINT noverdbee_save;"
        ))
        .await;
    session
        .run("ROLLBACK TO SAVEPOINT noverdbee_save; RELEASE SAVEPOINT noverdbee_save".to_string())
        .await;
    assert!(session.in_transaction().await);
    let count = rows(session, &format!("SELECT count(*) FROM {table}")).await;
    assert_eq!(count.rows, [cells(&[Some("1")])]);
    session.run("ROLLBACK".to_string()).await;
    session.run(format!("DROP TABLE {table}")).await;
}

#[tokio::test]
async fn sqlite_a_failed_save_keeps_the_open_transaction() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");
    a_failed_save_keeps_the_open_transaction(&session, "saves").await;
}

/// Needs a server, as above.
#[tokio::test]
async fn postgres_a_failed_save_keeps_the_open_transaction() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let session = Session::open(&pg_record(&url)).await.expect("connect");
    let table = format!("noverdbee_saves_{}", std::process::id());
    a_failed_save_keeps_the_open_transaction(&session, &table).await;
}

#[tokio::test]
async fn read_all_reads_past_the_row_limit_with_the_filter_and_order() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");
    // 12,001 rows: more than one page and more than ROW_LIMIT.
    session
        .run(
            "CREATE TABLE n (id INTEGER PRIMARY KEY);
             WITH RECURSIVE s(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM s WHERE i < 24001)
             INSERT INTO n SELECT i FROM s WHERE i % 2 = 0"
                .to_string(),
        )
        .await;
    let table = TableRef {
        schema: None,
        name: "n".to_string(),
    };
    let all = session
        .read_all(&table, "", Some(("id", Direction::Descending)))
        .await
        .expect("rows");
    assert_eq!(all.rows.len(), 12_001);
    assert_eq!(all.rows[0], cells(&[Some("24000")]));
    assert_eq!(all.rows[12_000], cells(&[Some("0")]));
    assert!(!all.truncated);

    let kept = session
        .read_all(&table, "id < 10", Some(("id", Direction::Ascending)))
        .await
        .expect("rows");
    assert_eq!(kept.rows.len(), 5);
}

#[tokio::test]
async fn sqlite_explains_each_statement_without_running_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let session = Session::open(&record(&directory, false))
        .await
        .expect("open");
    session
        .run("CREATE TABLE t (id INTEGER PRIMARY KEY, a INTEGER)".to_string())
        .await;
    let plans = super::plan::explain(
        &session,
        "SELECT * FROM t WHERE a = 1; DELETE FROM t",
        false,
    )
    .await;
    let plans: Vec<_> = plans.into_iter().map(|plan| plan.expect("plan")).collect();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].sql, "SELECT * FROM t WHERE a = 1");
    assert!(
        plans[0].roots[0].operation.starts_with("SCAN t"),
        "{:?}",
        plans[0].roots
    );
}

/// Needs a server, as above. Explain Analyze runs the SQL but never keeps its
/// writes, whether or not the user has a transaction open.
#[tokio::test]
async fn postgres_explain_analyze_rolls_back_its_writes() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let session = Session::open(&pg_record(&url)).await.expect("connect");
    let table = format!("noverdbee_explain_{}", std::process::id());
    session.run(format!("CREATE TABLE {table} (id int)")).await;
    let count = |session: &Session| {
        let sql = format!("SELECT count(*) FROM {table}");
        let session = session.clone();
        async move { rows(&session, &sql).await.rows }
    };

    let plans =
        super::plan::explain(&session, &format!("INSERT INTO {table} VALUES (1)"), true).await;
    let plan = plans[0].as_ref().expect("plan");
    assert!(plan.execution_ms.is_some());
    assert_eq!(plan.roots[0].operation, format!("Insert on {table}"));
    assert_eq!(plan.roots[0].children[0].actual_rows, Some(1.));
    assert!(!session.in_transaction().await);
    assert_eq!(count(&session).await, [cells(&[Some("0")])]);

    // A failure inside the block still ends it.
    let plans = super::plan::explain(&session, "SELECT 1 / 0", true).await;
    assert!(plans[0].is_err());
    assert!(!session.in_transaction().await);

    // In the user's transaction, only the explained write goes.
    session
        .run(format!("BEGIN; INSERT INTO {table} VALUES (7)"))
        .await;
    super::plan::explain(&session, &format!("DELETE FROM {table}"), true).await;
    assert!(session.in_transaction().await);
    assert_eq!(count(&session).await, [cells(&[Some("1")])]);
    session.run("ROLLBACK".to_string()).await;
    session.run(format!("DROP TABLE {table}")).await;
}

#[tokio::test]
async fn postgres_a_read_only_connection_refuses_writes() {
    let Ok(url) = std::env::var("NOVERDBEE_TEST_PG_URL") else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL is not set");
        return;
    };
    let record = DbConnectionRecord {
        read_only: true,
        ..pg_record(&url)
    };
    let session = Session::open(&record).await.expect("connect");
    let table = format!("noverdbee_ro_{}", std::process::id());
    let refused = session
        .execute_one(format!("CREATE TABLE {table} (id int)"))
        .await
        .expect_err("a read-only session refuses CREATE TABLE");
    assert!(refused.contains("read-only"), "{refused}");
    let (rows, _) = session
        .select_json("SELECT 1 AS one".to_string(), 10)
        .await
        .expect("reads still work");
    assert_eq!(rows.len(), 1);
}

/// Needs a server behind an SSH server: set `NOVERDBEE_TEST_PG_URL` to the
/// database as the SSH server sees it, and `NOVERDBEE_TEST_SSH` to
/// `user:password@host:port`. Skipped otherwise. Adds the SSH server to
/// ~/.ssh/known_hosts.
#[tokio::test]
async fn postgres_connects_through_an_ssh_tunnel() {
    let (Ok(url), Ok(ssh)) = (
        std::env::var("NOVERDBEE_TEST_PG_URL"),
        std::env::var("NOVERDBEE_TEST_SSH"),
    ) else {
        eprintln!("skipped: NOVERDBEE_TEST_PG_URL or NOVERDBEE_TEST_SSH is not set");
        return;
    };
    let (login, address) = ssh.split_once('@').expect("user:password@host:port");
    let (user, password) = login.split_once(':').expect("user:password");
    let (host, port) = address.rsplit_once(':').expect("host:port");
    let login = shared::SshLogin {
        host: host.to_string(),
        port: port.parse().expect("an SSH port"),
        username: user.to_string(),
        password: password.to_string(),
        ..Default::default()
    };
    let through = |login: shared::SshLogin| DbConnectionRecord {
        tunnel: super::tunnel::TUNNEL_SSH.to_string(),
        ssh: Some(login),
        ..pg_record(&url)
    };
    let record = through(login.clone());
    let session = Session::open(&record).await.expect("connect through SSH");
    let version = session.version().await.expect("version");
    assert!(version.starts_with("PostgreSQL "), "{version}");
    // The parse-only second connection goes through the tunnel too.
    assert!(session.check("SELECT 1".to_string()).await.is_none());

    // `NOVERDBEE_TEST_SSH_KEY`: a private key the SSH server accepts.
    if let Ok(key) = std::env::var("NOVERDBEE_TEST_SSH_KEY") {
        let with_key = through(shared::SshLogin {
            private_key: std::fs::read_to_string(key).expect("the key file"),
            password: String::new(),
            ..login.clone()
        });
        let session = Session::open(&with_key)
            .await
            .expect("connect with the key");
        session.version().await.expect("version");
    }

    let wrong = through(shared::SshLogin {
        password: "wrong".to_string(),
        ..login
    });
    let refused = Session::open(&wrong).await.expect_err("a wrong password");
    assert!(refused.contains("rejected"), "{refused}");
}

/// Needs a server in a Kubernetes cluster: set `NOVERDBEE_TEST_PG_URL` (its
/// host and port are not used), `NOVERDBEE_TEST_KUBECONFIG` to a kubeconfig
/// file, and `NOVERDBEE_TEST_KUBE` to the targets, as
/// `namespace/svc/name:port,namespace/pod/name:port`. Skipped otherwise.
#[tokio::test]
async fn postgres_connects_through_a_kubernetes_forward() {
    let (Ok(url), Ok(kubeconfig), Ok(targets)) = (
        std::env::var("NOVERDBEE_TEST_PG_URL"),
        std::env::var("NOVERDBEE_TEST_KUBECONFIG"),
        std::env::var("NOVERDBEE_TEST_KUBE"),
    ) else {
        eprintln!("skipped: NOVERDBEE_TEST_KUBECONFIG or NOVERDBEE_TEST_KUBE is not set");
        return;
    };
    let kube_config = std::fs::read_to_string(kubeconfig).expect("the kubeconfig file");
    for target in targets.split(',') {
        let (namespace, rest) = target.split_once('/').expect("namespace/kind/name:port");
        let (resource, port) = rest.rsplit_once(':').expect("kind/name:port");
        let record = DbConnectionRecord {
            tunnel: super::tunnel::TUNNEL_KUBERNETES.to_string(),
            kube_config: kube_config.clone(),
            kube_namespace: namespace.to_string(),
            kube_resource: resource.to_string(),
            port: port.parse().expect("a port"),
            ..pg_record(&url)
        };
        let session = Session::open(&record)
            .await
            .unwrap_or_else(|error| panic!("{target}: {error}"));
        let version = session.version().await.expect("version");
        assert!(version.starts_with("PostgreSQL "), "{target}: {version}");
        // The parse-only second connection goes through the forward too.
        assert!(session.check("SELECT 1".to_string()).await.is_none());

        let missing = DbConnectionRecord {
            kube_resource: "svc/noverdbee-no-such-service".to_string(),
            ..record
        };
        let refused = Session::open(&missing).await.expect_err("no such service");
        assert!(refused.starts_with("cannot find svc/"), "{refused}");
    }
}
