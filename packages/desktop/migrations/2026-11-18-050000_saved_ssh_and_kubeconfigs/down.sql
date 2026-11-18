ALTER TABLE db_connections ADD COLUMN ssh_host TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_port INTEGER NOT NULL DEFAULT 22;
ALTER TABLE db_connections ADD COLUMN ssh_user TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_password TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN ssh_key_path TEXT NOT NULL DEFAULT '';
ALTER TABLE db_connections ADD COLUMN kube_config TEXT NOT NULL DEFAULT '';

UPDATE db_connections
SET ssh_host = ssh_hosts.host,
    ssh_port = ssh_hosts.port,
    ssh_user = ssh_hosts.username,
    ssh_password = ssh_hosts.password
FROM ssh_hosts
WHERE ssh_hosts.id = db_connections.ssh_host_id;

UPDATE db_connections
SET kube_config = kubeconfigs.config
FROM kubeconfigs
WHERE kubeconfigs.id = db_connections.kubeconfig_id;

DROP INDEX db_connections_kubeconfig_id_idx;
DROP INDEX db_connections_ssh_host_id_idx;
ALTER TABLE db_connections DROP COLUMN kubeconfig_id;
ALTER TABLE db_connections DROP COLUMN ssh_host_id;
DROP TABLE kubeconfigs;
DROP TABLE ssh_hosts;
DROP TABLE ssh_keys;
