//! SQL the user ran, per saved connection, for the editor's History list.

use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use orm::models::{NewQueryHistory, QueryHistory};
use orm::schema::query_history;
use uuid::Uuid;

use super::{internal_error, run_db, DbPool};

/// Entries kept per connection; older ones are dropped as new ones arrive.
const KEEP: i64 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub sql: String,
    /// When it last ran, in UTC.
    pub ran_at: NaiveDateTime,
}

/// Records that `sql` ran on `connection_id`. Running the same SQL again moves
/// its entry to the top instead of adding a second one.
pub async fn history_add(
    connection_id: String,
    sql: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let sql = sql.trim().to_string();
        if sql.is_empty() {
            return Ok(());
        }
        let now = Utc::now().naive_utc();
        connection
            .transaction(|connection| {
                let mine =
                    query_history::table.filter(query_history::connection_id.eq(&connection_id));
                let moved = diesel::update(mine.filter(query_history::sql.eq(&sql)))
                    .set(query_history::updated_at.eq(now))
                    .execute(connection)?;
                if moved == 0 {
                    diesel::insert_into(query_history::table)
                        .values(&NewQueryHistory {
                            id: Uuid::new_v4().to_string(),
                            connection_id: connection_id.clone(),
                            sql,
                            created_at: now,
                            updated_at: now,
                        })
                        .execute(connection)?;
                }
                let old: Vec<String> = mine
                    .order(query_history::updated_at.desc())
                    .offset(KEEP)
                    .limit(i64::MAX)
                    .select(query_history::id)
                    .load(connection)?;
                diesel::delete(query_history::table.filter(query_history::id.eq_any(old)))
                    .execute(connection)
            })
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}

/// The connection's history, most recent first.
pub async fn history_list(
    connection_id: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<Vec<HistoryEntry>, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        query_history::table
            .filter(query_history::connection_id.eq(&connection_id))
            .order(query_history::updated_at.desc())
            .select(QueryHistory::as_select())
            .load(connection)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| HistoryEntry {
                        sql: row.sql,
                        ran_at: row.updated_at,
                    })
                    .collect()
            })
            .map_err(internal_error)
    })
    .await
}
