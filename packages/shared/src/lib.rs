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
    /// Empty connects directly, `ssh` goes through the saved SSH host, `kubernetes`
    /// to a pod in the saved kubeconfig's cluster. `host` and `port` are the
    /// target either way; kubernetes has no `host`.
    pub tunnel: String,
    pub ssh_host_id: Option<String>,
    pub kubeconfig_id: Option<String>,
    /// The kubeconfig context; empty uses its current context.
    pub kube_context: String,
    pub kube_namespace: String,
    /// What to forward to, such as `svc/postgres` or `pod/db-0`.
    pub kube_resource: String,
    /// The SSH host with its key, looked up by [`Self::resolve`]; not saved.
    pub ssh: Option<SshLogin>,
    /// The kubeconfig's YAML, looked up by [`Self::resolve`]; not saved.
    pub kube_config: String,
}

impl DbConnectionRecord {
    /// Fills in what the tunnel signs in with from the saved host, its key
    /// and the saved kubeconfig this connection points at.
    pub fn resolve(
        &mut self,
        hosts: &[SshHostRecord],
        keys: &[SshKeyRecord],
        kubeconfigs: &[KubeconfigRecord],
    ) {
        self.ssh = hosts
            .iter()
            .find(|host| Some(&host.id) == self.ssh_host_id.as_ref())
            .map(|host| {
                let key = keys
                    .iter()
                    .find(|key| Some(&key.id) == host.key_id.as_ref());
                SshLogin {
                    host: host.host.clone(),
                    port: host.port,
                    username: host.username.clone(),
                    password: host.password.clone(),
                    private_key: key.map(|key| key.private_key.clone()).unwrap_or_default(),
                    passphrase: key.map(|key| key.passphrase.clone()).unwrap_or_default(),
                }
            });
        self.kube_config = kubeconfigs
            .iter()
            .find(|kubeconfig| Some(&kubeconfig.id) == self.kubeconfig_id.as_ref())
            .map(|kubeconfig| kubeconfig.config.clone())
            .unwrap_or_default();
    }
}

/// A saved SSH private key. `id` is empty until it has been saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshKeyRecord {
    pub id: String,
    pub name: String,
    pub private_key: String,
    /// Empty when the key is not encrypted.
    pub passphrase: String,
    /// The key's SHA-256 fingerprint, set when it is saved.
    pub fingerprint: String,
}

/// A saved SSH server that connections tunnel through. `id` is empty until
/// it has been saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshHostRecord {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    /// Tried after the key, or alone when there is no key.
    pub password: String,
    pub key_id: Option<String>,
}

/// A saved kubeconfig, as YAML. `id` is empty until it has been saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubeconfigRecord {
    pub id: String,
    pub name: String,
    pub config: String,
}

/// What an SSH tunnel signs in with: a saved host and its key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshLogin {
    pub host: String,
    pub port: i32,
    pub username: String,
    pub password: String,
    /// Empty signs in with the password alone.
    pub private_key: String,
    pub passphrase: String,
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
