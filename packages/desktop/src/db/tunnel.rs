//! Port forwards a connection opens before it connects: through an SSH
//! server, or with `kubectl port-forward`. The database client then connects
//! to the forward's port on 127.0.0.1.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::ssh_key::{Algorithm, PublicKey};
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg};
use shared::DbConnectionRecord;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

pub const TUNNEL_SSH: &str = "ssh";
pub const TUNNEL_KUBECTL: &str = "kubectl";

/// How long a tunnel may take to come up.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// An open forward. Dropping it closes the forward.
#[derive(Debug)]
pub struct Tunnel {
    /// The local port that reaches the database.
    pub port: u16,
    task: JoinHandle<()>,
    /// kubectl, killed on drop.
    _child: Option<Child>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The tunnel `record` asks for, or `None` when it connects directly.
pub async fn open(record: &DbConnectionRecord) -> Result<Option<Tunnel>, String> {
    let port = u16::try_from(record.port).map_err(|_| "the port is out of range".to_string())?;
    match record.tunnel.as_str() {
        "" => Ok(None),
        TUNNEL_SSH => tokio::time::timeout(START_TIMEOUT, ssh(record, port))
            .await
            .map_err(|_| "the SSH server did not answer within 30 seconds".to_string())?
            .map(Some),
        TUNNEL_KUBECTL => kubectl(record, port).await.map(Some),
        other => Err(format!("unsupported tunnel: {other}")),
    }
}

struct SshHandler {
    host: String,
    port: u16,
}

impl client::Handler for SshHandler {
    type Error = russh::Error;

    /// Checks the key against ~/.ssh/known_hosts. A new host is added, as
    /// OpenSSH's `StrictHostKeyChecking=accept-new` does; a changed key fails.
    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        if !russh::keys::check_known_hosts(&self.host, self.port, key)? {
            russh::keys::known_hosts::learn_known_hosts(&self.host, self.port, key)?;
        }
        Ok(true)
    }
}

async fn ssh(record: &DbConnectionRecord, target_port: u16) -> Result<Tunnel, String> {
    let ssh_port =
        u16::try_from(record.ssh_port).map_err(|_| "the SSH port is out of range".to_string())?;
    let config = Arc::new(client::Config {
        // Notice a dead link, as the PostgreSQL keepalive does.
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let handler = SshHandler {
        host: record.ssh_host.clone(),
        port: ssh_port,
    };
    let mut handle = client::connect(config, (record.ssh_host.as_str(), ssh_port), handler)
        .await
        .map_err(|error| match error {
            russh::Error::Keys(russh::keys::Error::KeyChanged { line }) => format!(
                "the SSH host key of {} changed (known_hosts line {line}); \
                 remove that line if the change is expected",
                record.ssh_host
            ),
            error => format!("cannot reach the SSH server: {error}"),
        })?;
    authenticate(&mut handle, record).await?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| format!("cannot open a local port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot open a local port: {error}"))?
        .port();
    let handle = Arc::new(handle);
    let target_host = record.host.clone();
    let task = tokio::spawn(async move {
        // ponytail: the forward tasks outlive the listener until their own
        // streams end; track them in a JoinSet if that ever matters.
        while let Ok((mut local, peer)) = listener.accept().await {
            let handle = handle.clone();
            let target_host = target_host.clone();
            tokio::spawn(async move {
                let channel = match handle
                    .channel_open_direct_tcpip(
                        target_host,
                        target_port.into(),
                        peer.ip().to_string(),
                        peer.port().into(),
                    )
                    .await
                {
                    Ok(channel) => channel,
                    Err(error) => {
                        tracing::warn!(error = %error, "SSH server refused the forward");
                        return;
                    }
                };
                let mut remote = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
            });
        }
    });
    Ok(Tunnel {
        port,
        task,
        _child: None,
    })
}

/// Signs in with the key when one is set (its passphrase is the password),
/// falling back to the password; with no key, with the password.
async fn authenticate(
    handle: &mut Handle<SshHandler>,
    record: &DbConnectionRecord,
) -> Result<(), String> {
    let user = record.ssh_user.clone();
    let key_path = record.ssh_key_path.trim();
    if !key_path.is_empty() {
        let key = load_key(key_path, &record.ssh_password)?;
        let hash = if matches!(key.algorithm(), Algorithm::Rsa { .. }) {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| format!("SSH sign-in failed: {error}"))?
                .flatten()
        } else {
            None
        };
        let result = handle
            .authenticate_publickey(
                user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await
            .map_err(|error| format!("SSH sign-in failed: {error}"))?;
        if result.success() {
            return Ok(());
        }
        if record.ssh_password.is_empty() {
            return Err("the SSH server rejected the key".to_string());
        }
    }
    let result = handle
        .authenticate_password(user, record.ssh_password.clone())
        .await
        .map_err(|error| format!("SSH sign-in failed: {error}"))?;
    if result.success() {
        Ok(())
    } else {
        Err("the SSH server rejected the sign-in".to_string())
    }
}

fn load_key(path: &str, passphrase: &str) -> Result<russh::keys::PrivateKey, String> {
    let path = match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => path.into(),
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read the SSH key {}: {error}", path.display()))?;
    let passphrase = (!passphrase.is_empty()).then_some(passphrase);
    decode_secret_key(&text, passphrase).map_err(|error| match error {
        russh::keys::Error::KeyIsEncrypted => {
            "the SSH key is encrypted; enter its passphrase as the SSH password".to_string()
        }
        error => format!("cannot read the SSH key: {error}"),
    })
}

/// Runs `kubectl port-forward` on a free local port. It runs in an
/// interactive login shell, which reads ~/.zprofile and ~/.zshrc, so kubectl,
/// its auth plugins and KUBECONFIG are what the terminal has: an app opened
/// from the Finder gets none of the terminal's environment.
async fn kubectl(record: &DbConnectionRecord, target_port: u16) -> Result<Tunnel, String> {
    let mut args = vec!["port-forward".to_string()];
    if !record.kube_context.is_empty() {
        args.extend(["--context".to_string(), record.kube_context.clone()]);
    }
    if !record.kube_namespace.is_empty() {
        args.extend(["--namespace".to_string(), record.kube_namespace.clone()]);
    }
    args.extend([
        "--address".to_string(),
        "127.0.0.1".to_string(),
        record.kube_resource.clone(),
        // No local port: kubectl picks a free one and prints it.
        format!(":{target_port}"),
    ]);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut child = Command::new(shell)
        .args(["-lic", "exec kubectl \"$@\"", "kubectl"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("cannot run kubectl: {error}"))?;
    let mut stdout = BufReader::new(child.stdout.take().expect("piped")).lines();
    let mut stderr = child.stderr.take().expect("piped");
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let started = tokio::time::timeout(START_TIMEOUT, async {
        while let Ok(Some(line)) = stdout.next_line().await {
            if let Some(port) = forwarded_port(&line) {
                return Some(port);
            }
        }
        None
    })
    .await;
    match started {
        Ok(Some(port)) => Ok(Tunnel {
            port,
            // kubectl prints a line per connection; keep reading so it never
            // blocks on a full pipe.
            task: tokio::spawn(async move { while let Ok(Some(_)) = stdout.next_line().await {} }),
            _child: Some(child),
        }),
        Ok(None) => {
            let _ = child.wait().await;
            let errors = errors.await.unwrap_or_default();
            Err(format!("kubectl port-forward failed: {}", errors.trim()))
        }
        Err(_) => Err("kubectl port-forward did not start within 30 seconds".to_string()),
    }
}

/// The local port in kubectl's `Forwarding from 127.0.0.1:54321 -> 5432`.
fn forwarded_port(line: &str) -> Option<u16> {
    let rest = line.strip_prefix("Forwarding from 127.0.0.1:")?;
    rest.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{forwarded_port, open, TUNNEL_KUBECTL};
    use shared::DbConnectionRecord;

    /// Runs kubectl, or fails to find it; either way the error says so.
    #[tokio::test]
    async fn a_failed_kubectl_says_why() {
        let record = DbConnectionRecord {
            port: 5432,
            tunnel: TUNNEL_KUBECTL.to_string(),
            kube_context: "noverdbee-no-such-context".to_string(),
            kube_resource: "svc/postgres".to_string(),
            ..Default::default()
        };
        let error = open(&record).await.expect_err("no such context");
        eprintln!("{error}");
        assert!(
            error.starts_with("kubectl port-forward failed: "),
            "{error}"
        );
    }

    #[test]
    fn reads_the_port_kubectl_picked() {
        assert_eq!(
            forwarded_port("Forwarding from 127.0.0.1:54321 -> 5432"),
            Some(54321)
        );
        assert_eq!(forwarded_port("Forwarding from [::1]:54321 -> 5432"), None);
        assert_eq!(forwarded_port("Handling connection for 5432"), None);
    }
}
