use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::ssh_hosts;

#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = ssh_hosts)]
pub struct SshHost {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub password: String,
    pub ssh_key_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = ssh_hosts)]
pub struct NewSshHost {
    pub id: String,
    pub created_at: NaiveDateTime,
    #[diesel(embed)]
    pub fields: UpdateSshHost,
}

/// Every editable column. A `None` key clears the key.
#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = ssh_hosts, treat_none_as_null = true)]
pub struct UpdateSshHost {
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub password: String,
    pub ssh_key_id: Option<String>,
    pub updated_at: NaiveDateTime,
}
