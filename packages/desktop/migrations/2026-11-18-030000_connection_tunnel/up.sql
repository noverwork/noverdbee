-- How the connection reaches its server: '' directly, 'ssh' through an SSH
-- server, 'kubectl' through `kubectl port-forward`. The ssh_* and kube_*
-- columns describe the tunnel; the connection's host and port are the target.
ALTER TABLE db_connections ADD COLUMN tunnel TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_host TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_port INTEGER NOT NULL DEFAULT 22;
ALTER TABLE db_connections ADD COLUMN ssh_user TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_password TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_key_path TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN kube_context TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN kube_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN kube_resource TEXT NOT NULL DEFAULT '';
