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
    pub mcp: McpAccess,
    /// Empty connects directly, `ssh` goes through an SSH server, `kubernetes` to a
    /// pod in a cluster. `host` and `port` are the target either way;
    /// kubernetes has no `host`.
    pub tunnel: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    pub ssh_user: String,
    /// The SSH password, or the key's passphrase when a key is set.
    pub ssh_password: String,
    pub ssh_key_path: String,
    /// The kubeconfig context; empty uses its current context.
    pub kube_context: String,
    pub kube_namespace: String,
    /// What to forward to, such as `svc/postgres` or `pod/db-0`.
    pub kube_resource: String,
    /// The kubeconfig the Kubernetes tunnel signs in with, as YAML.
    pub kube_config: String,
}

/// The MCP tools AI clients may use on a connection. All off keeps the
/// connection out of MCP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAccess {
    pub select: bool,
    pub insert: bool,
    pub update: bool,
    pub delete: bool,
}

impl McpAccess {
    pub fn any(self) -> bool {
        self.select || self.insert || self.update || self.delete
    }
}
