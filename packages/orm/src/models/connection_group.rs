use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::connection_groups;

#[derive(Debug, Clone, Queryable, Selectable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = connection_groups)]
pub struct ConnectionGroup {
    pub id: String,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = connection_groups)]
pub struct NewConnectionGroup {
    pub id: String,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = connection_groups)]
pub struct UpdateConnectionGroup {
    pub name: String,
    pub updated_at: NaiveDateTime,
}
