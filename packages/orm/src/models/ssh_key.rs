use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::ssh_keys;

#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = ssh_keys)]
pub struct SshKey {
    pub id: String,
    pub name: String,
    pub private_key: String,
    pub passphrase: String,
    pub fingerprint: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = ssh_keys)]
pub struct NewSshKey {
    pub id: String,
    pub created_at: NaiveDateTime,
    #[diesel(embed)]
    pub fields: UpdateSshKey,
}

/// Every editable column.
#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = ssh_keys, treat_none_as_null = true)]
pub struct UpdateSshKey {
    pub name: String,
    pub private_key: String,
    pub passphrase: String,
    pub fingerprint: String,
    pub updated_at: NaiveDateTime,
}
