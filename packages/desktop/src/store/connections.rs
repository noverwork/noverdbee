use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use orm::models::{DbConnection, NewConnectionGroup, NewDbConnection, UpdateDbConnection};
use orm::schema::{connection_groups, db_connections};
use shared::{DbConnectionRecord, Environment, McpAccess};
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

/// Creates the connection when its `id` is empty, otherwise updates it. The
/// connection joins the group called `group_name`, which is created if it is
/// new; an empty name leaves it ungrouped. `record.group_id` is ignored.
pub async fn connection_save(
    record: DbConnectionRecord,
    group_name: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<DbConnectionRecord, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let name = record.name.trim().to_string();
        if name.is_empty() {
            return Err("connection name is required".to_string());
        }
        connection
            .transaction(|connection| {
                let now = Utc::now().naive_utc();
                let fields = UpdateDbConnection {
                    name,
                    driver: record.driver,
                    group_id: group_id(connection, group_name.trim())?,
                    host: record.host,
                    port: record.port,
                    database: record.database,
                    username: record.username,
                    password: record.password,
                    ssl_mode: record.ssl_mode,
                    path: record.path,
                    read_only: record.read_only,
                    environment: record.environment.as_str().to_string(),
                    mcp_select: record.mcp.select,
                    mcp_insert: record.mcp.insert,
                    mcp_update: record.mcp.update,
                    mcp_delete: record.mcp.delete,
                    tunnel: record.tunnel,
                    ssh_host: record.ssh_host,
                    ssh_port: record.ssh_port,
                    ssh_user: record.ssh_user,
                    ssh_password: record.ssh_password,
                    ssh_key_path: record.ssh_key_path,
                    kube_context: record.kube_context,
                    kube_namespace: record.kube_namespace,
                    kube_resource: record.kube_resource,
                    kube_config: record.kube_config,
                    updated_at: now,
                };
                let saved = if record.id.is_empty() {
                    diesel::insert_into(db_connections::table)
                        .values(&NewDbConnection {
                            id: Uuid::new_v4().to_string(),
                            created_at: now,
                            fields,
                        })
                        .get_result::<DbConnection>(connection)?
                } else {
                    diesel::update(db_connections::table.find(record.id))
                        .set(&fields)
                        .get_result::<DbConnection>(connection)?
                };
                drop_empty_groups(connection)?;
                Ok(to_record(saved))
            })
            .map_err(internal_error)
    })
    .await
}

/// The id of the group called `name`, creating it if needed.
fn group_id(connection: &mut SqliteConnection, name: &str) -> QueryResult<Option<String>> {
    if name.is_empty() {
        return Ok(None);
    }
    let existing = connection_groups::table
        .filter(connection_groups::name.eq(name))
        .select(connection_groups::id)
        .first::<String>(connection)
        .optional()?;
    if existing.is_some() {
        return Ok(existing);
    }
    let now = Utc::now().naive_utc();
    let id = Uuid::new_v4().to_string();
    diesel::insert_into(connection_groups::table)
        .values(&NewConnectionGroup {
            id: id.clone(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
        })
        .execute(connection)?;
    Ok(Some(id))
}

/// A group exists only while a connection is in it.
pub(super) fn drop_empty_groups(connection: &mut SqliteConnection) -> QueryResult<usize> {
    diesel::sql_query(
        "DELETE FROM connection_groups WHERE id NOT IN \
         (SELECT group_id FROM db_connections WHERE group_id IS NOT NULL)",
    )
    .execute(connection)
}

pub async fn connection_delete(
    id: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        connection
            .transaction(|connection| {
                diesel::delete(db_connections::table.find(id)).execute(connection)?;
                drop_empty_groups(connection)
            })
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
        environment: Environment::parse(&row.environment),
        mcp: McpAccess {
            select: row.mcp_select,
            insert: row.mcp_insert,
            update: row.mcp_update,
            delete: row.mcp_delete,
        },
        tunnel: row.tunnel,
        ssh_host: row.ssh_host,
        ssh_port: row.ssh_port,
        ssh_user: row.ssh_user,
        ssh_password: row.ssh_password,
        ssh_key_path: row.ssh_key_path,
        kube_context: row.kube_context,
        kube_namespace: row.kube_namespace,
        kube_resource: row.kube_resource,
        kube_config: row.kube_config,
    }
}
