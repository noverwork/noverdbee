//! Port forwards a connection opens before it connects: through an SSH
//! server, or to a pod in a Kubernetes cluster. The database client then connects
//! to the forward's port on 127.0.0.1.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use russh::client::{self, Handle};
use russh::keys::ssh_key::Algorithm;
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use shared::DbConnectionRecord;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub const TUNNEL_SSH: &str = "ssh";
pub const TUNNEL_KUBERNETES: &str = "kubernetes";

/// How long a tunnel may take to come up.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// An open forward. Dropping it closes the forward.
#[derive(Debug)]
pub struct Tunnel {
    /// The local port that reaches the database.
    pub port: u16,
    task: JoinHandle<()>,
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
        TUNNEL_KUBERNETES => kube(record, port).await.map(Some),
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
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = key.public_key();
        if !russh::keys::check_known_hosts(&self.host, self.port, &key)? {
            russh::keys::known_hosts::learn_known_hosts(&self.host, self.port, &key)?;
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
    Ok(Tunnel { port, task })
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

/// Forwards a local port to a pod, as `kubectl port-forward` does, with the
/// kubeconfig saved on the connection. Nothing is read from the machine's
/// own kubeconfig or environment.
async fn kube(record: &DbConnectionRecord, target_port: u16) -> Result<Tunnel, String> {
    let kubeconfig = Kubeconfig::from_yaml(&record.kube_config)
        .map_err(|error| format!("cannot read the kubeconfig: {error}"))?;
    let options = KubeConfigOptions {
        context: (!record.kube_context.is_empty()).then(|| record.kube_context.clone()),
        ..Default::default()
    };
    let config = kube::Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .map_err(|error| format!("cannot use the kubeconfig: {error}"))?;
    let namespace = if record.kube_namespace.is_empty() {
        config.default_namespace.clone()
    } else {
        record.kube_namespace.clone()
    };
    let client = kube::Client::try_from(config)
        .map_err(|error| format!("cannot use the kubeconfig: {error}"))?;
    let (pod, pod_port) = tokio::time::timeout(
        START_TIMEOUT,
        find_pod(&client, &namespace, &record.kube_resource, target_port),
    )
    .await
    .map_err(|_| "the Kubernetes cluster did not answer within 30 seconds".to_string())??;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| format!("cannot open a local port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot open a local port: {error}"))?
        .port();
    let pods: Api<Pod> = Api::namespaced(client, &namespace);
    let task = tokio::spawn(async move {
        while let Ok((mut local, _)) = listener.accept().await {
            let (pods, pod) = (pods.clone(), pod.clone());
            tokio::spawn(async move {
                let mut forwarder = match pods.portforward(&pod, &[pod_port]).await {
                    Ok(forwarder) => forwarder,
                    Err(error) => {
                        tracing::warn!(error = %error, pod, "Kubernetes refused the forward");
                        return;
                    }
                };
                let Some(mut remote) = forwarder.take_stream(pod_port) else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
            });
        }
    });
    Ok(Tunnel { port, task })
}

/// The pod behind `resource` and the port on it that `port` means. `resource`
/// is what kubectl takes: `pod/name` or a bare pod name, `svc/name`,
/// `deploy/name` or `sts/name`. A service's port becomes its target port.
async fn find_pod(
    client: &kube::Client,
    namespace: &str,
    resource: &str,
    port: u16,
) -> Result<(String, u16), String> {
    let (kind, name) = resource.split_once('/').unwrap_or(("pod", resource));
    let failed = |error: kube::Error| format!("cannot find {resource}: {error}");
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let mut target = IntOrString::Int(port.into());
    let selector = match kind {
        "pod" | "pods" | "po" => {
            pods.get(name).await.map_err(failed)?;
            return Ok((name.to_string(), port));
        }
        "svc" | "service" | "services" => {
            let service = Api::<Service>::namespaced(client.clone(), namespace)
                .get(name)
                .await
                .map_err(failed)?;
            let spec = service.spec.unwrap_or_default();
            let service_port = spec
                .ports
                .unwrap_or_default()
                .into_iter()
                .find(|service_port| service_port.port == i32::from(port))
                .ok_or_else(|| format!("{resource} has no port {port}"))?;
            if let Some(target_port) = service_port.target_port {
                target = target_port;
            }
            spec.selector
        }
        "deploy" | "deployment" | "deployments" => {
            Api::<Deployment>::namespaced(client.clone(), namespace)
                .get(name)
                .await
                .map_err(failed)?
                .spec
                .selector
                .match_labels
        }
        "sts" | "statefulset" | "statefulsets" => {
            Api::<StatefulSet>::namespaced(client.clone(), namespace)
                .get(name)
                .await
                .map_err(failed)?
                .spec
                .selector
                .match_labels
        }
        other => {
            return Err(format!(
                "cannot forward to a {other}; use pod, svc, deploy or sts"
            ))
        }
    };
    let selector = selector
        .filter(|labels| !labels.is_empty())
        .ok_or_else(|| format!("{resource} selects no pods"))?
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    let pod = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(failed)?
        .into_iter()
        .find(|pod| {
            pod.metadata.deletion_timestamp.is_none()
                && pod
                    .status
                    .as_ref()
                    .and_then(|status| status.phase.as_deref())
                    == Some("Running")
        })
        .ok_or_else(|| format!("{resource} has no running pod"))?;
    let port = pod_port(&pod, &target)
        .ok_or_else(|| format!("the pod of {resource} has no port {target:?}"))?;
    Ok((pod.metadata.name.unwrap_or_default(), port))
}

/// `target` as a number: itself, or the container port with that name.
fn pod_port(pod: &Pod, target: &IntOrString) -> Option<u16> {
    let number = match target {
        IntOrString::Int(number) => *number,
        IntOrString::String(name) => {
            pod.spec
                .as_ref()?
                .containers
                .iter()
                .flat_map(|container| container.ports.iter().flatten())
                .find(|port| port.name.as_deref() == Some(name))?
                .container_port
        }
    };
    u16::try_from(number).ok()
}

#[cfg(test)]
mod tests {
    use super::{open, pod_port, TUNNEL_KUBERNETES};
    use k8s_openapi::api::core::v1::{Container, ContainerPort, Pod, PodSpec};
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    use shared::DbConnectionRecord;

    #[test]
    fn a_named_target_port_is_the_container_port_with_that_name() {
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    ports: Some(vec![ContainerPort {
                        name: Some("postgres".to_string()),
                        container_port: 5432,
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let named = |name: &str| IntOrString::String(name.to_string());
        assert_eq!(pod_port(&pod, &named("postgres")), Some(5432));
        assert_eq!(pod_port(&pod, &named("metrics")), None);
        assert_eq!(pod_port(&pod, &IntOrString::Int(6432)), Some(6432));
    }

    #[tokio::test]
    async fn a_kubeconfig_that_cannot_be_read_says_so() {
        let record = DbConnectionRecord {
            port: 5432,
            tunnel: TUNNEL_KUBERNETES.to_string(),
            kube_config: "clusters: 3".to_string(),
            kube_resource: "svc/postgres".to_string(),
            ..Default::default()
        };
        let error = open(&record).await.expect_err("not a kubeconfig");
        assert!(error.starts_with("cannot read the kubeconfig: "), "{error}");
    }
}
