use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::query_history;

#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = query_history)]
pub struct QueryHistory {
    pub id: String,
    pub connection_id: String,
    pub sql: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = query_history)]
pub struct NewQueryHistory {
    pub id: String,
    pub connection_id: String,
    pub sql: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
