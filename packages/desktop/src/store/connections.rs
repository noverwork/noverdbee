use chrono::Utc;
use diesel::prelude::*;
use orm::models::{DbConnection, NewDbConnection, UpdateDbConnection};
use orm::schema::db_connections;
use shared::DbConnectionRecord;
use uuid::Uuid;

use super::{internal_error, run_db, DbPool};

pub async fn connection_list(
    pool: crate::app::State<'_, DbPool>,
) -> Result<Vec<DbConnectionRecord>, String> {
    let pool = pool.inner().clone();
    run_db(pool, |connection| {
        db_connections::table
            .order(db_connections::name.asc())
            .load::<DbConnection>(connection)
            .map(|rows| rows.into_iter().map(to_record).collect())
            .map_err(internal_error)
    })
    .await
}

/// Creates the connection when its `id` is empty, otherwise updates it.
pub async fn connection_save(
    record: DbConnectionRecord,
    pool: crate::app::State<'_, DbPool>,
) -> Result<DbConnectionRecord, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let name = record.name.trim().to_string();
        if name.is_empty() {
            return Err("connection name is required".to_string());
        }
        let now = Utc::now().naive_utc();
        let fields = UpdateDbConnection {
            name,
            driver: record.driver,
            group_id: record.group_id,
            host: record.host,
            port: record.port,
            database: record.database,
            username: record.username,
            password: record.password,
            ssl_mode: record.ssl_mode,
            path: record.path,
            read_only: record.read_only,
            updated_at: now,
        };
        let saved = if record.id.is_empty() {
            diesel::insert_into(db_connections::table)
                .values(&NewDbConnection {
                    id: Uuid::new_v4().to_string(),
                    created_at: now,
                    fields,
                })
                .get_result::<DbConnection>(connection)
        } else {
            diesel::update(db_connections::table.find(record.id))
                .set(&fields)
                .get_result::<DbConnection>(connection)
        };
        saved.map(to_record).map_err(internal_error)
    })
    .await
}

pub async fn connection_delete(
    id: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        diesel::delete(db_connections::table.find(id))
            .execute(connection)
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}

fn to_record(row: DbConnection) -> DbConnectionRecord {
    DbConnectionRecord {
        id: row.id,
        name: row.name,
        driver: row.driver,
        group_id: row.group_id,
        host: row.host,
        port: row.port,
        database: row.database,
        username: row.username,
        password: row.password,
        ssl_mode: row.ssl_mode,
        path: row.path,
        read_only: row.read_only,
    }
}
