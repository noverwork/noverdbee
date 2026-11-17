use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::db_connections;

#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = db_connections)]
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub group_id: Option<String>,
    pub host: String,
    pub port: i32,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: String,
    pub path: String,
    pub read_only: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// `local`, `development`, `staging` or `production`.
    pub environment: String,
    pub mcp_select: bool,
    pub mcp_insert: bool,
    pub mcp_update: bool,
    pub mcp_delete: bool,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = db_connections)]
pub struct NewDbConnection {
    pub id: String,
    pub created_at: NaiveDateTime,
    #[diesel(embed)]
    pub fields: UpdateDbConnection,
}

/// Every editable column. A `None` group clears the group.
#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = db_connections, treat_none_as_null = true)]
pub struct UpdateDbConnection {
    pub name: String,
    pub driver: String,
    pub group_id: Option<String>,
    pub host: String,
    pub port: i32,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: String,
    pub path: String,
    pub read_only: bool,
    pub environment: String,
    pub mcp_select: bool,
    pub mcp_insert: bool,
    pub mcp_update: bool,
    pub mcp_delete: bool,
    pub updated_at: NaiveDateTime,
}
