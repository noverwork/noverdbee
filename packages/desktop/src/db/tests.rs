use shared::DbConnectionRecord;
use tempfile::TempDir;

use super::sql::{self, Direction};
use super::{ObjectKind, Outcome, ResultSet, Session, SessionManager, TableRef, DRIVER_SQLITE};

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
