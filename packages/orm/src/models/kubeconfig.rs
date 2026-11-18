use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::kubeconfigs;

#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = kubeconfigs)]
pub struct Kubeconfig {
    pub id: String,
    pub name: String,
    pub config: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = kubeconfigs)]
pub struct NewKubeconfig {
    pub id: String,
    pub created_at: NaiveDateTime,
    #[diesel(embed)]
    pub fields: UpdateKubeconfig,
}

/// Every editable column.
#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = kubeconfigs, treat_none_as_null = true)]
pub struct UpdateKubeconfig {
    pub name: String,
    pub config: String,
    pub updated_at: NaiveDateTime,
}
