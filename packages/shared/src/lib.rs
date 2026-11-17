use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRecord {
    pub id: String,
    pub name: String,
}

/// A saved database connection. `id` is empty until it has been saved.
/// `driver` is `sqlite` or `postgres`; fields the driver does not use are empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbConnectionRecord {
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
}
