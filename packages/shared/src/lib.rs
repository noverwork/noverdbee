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

/// Which environment a connection reaches. It sets the connection's colour,
/// so production always looks like production.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Environment {
    Local,
    #[default]
    Development,
    Staging,
    Production,
}

impl Environment {
    pub const ALL: [Environment; 4] = [
        Environment::Local,
        Environment::Development,
        Environment::Staging,
        Environment::Production,
    ];

    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Local => "local",
            Environment::Development => "development",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }

    /// The environment stored as `name`; an unknown name is development.
    pub fn parse(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|environment| environment.as_str() == name)
            .unwrap_or_default()
    }
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
    pub environment: Environment,
}
