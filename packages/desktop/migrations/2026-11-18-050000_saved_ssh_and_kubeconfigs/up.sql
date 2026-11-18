-- SSH keys, SSH hosts and kubeconfigs are saved once and picked by the
-- connections that tunnel through them, instead of typed into each connection.
CREATE TABLE ssh_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    private_key TEXT NOT NULL,
    passphrase TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE ssh_hosts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22,
    username TEXT NOT NULL,
    password TEXT NOT NULL DEFAULT '',
    ssh_key_id TEXT REFERENCES ssh_keys (id) ON DELETE SET NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX ssh_hosts_ssh_key_id_idx ON ssh_hosts (ssh_key_id);

CREATE TABLE kubeconfigs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE db_connections ADD COLUMN ssh_host_id TEXT REFERENCES ssh_hosts (id) ON DELETE SET NULL;
ALTER TABLE db_connections ADD COLUMN kubeconfig_id TEXT REFERENCES kubeconfigs (id) ON DELETE SET NULL;

-- Each SSH server a connection named becomes a saved host. A key was a file
-- path and its passphrase shared the password field, so neither carries
-- over: the key has to be added under SSH Keys and picked on the host.
INSERT INTO ssh_hosts (id, name, host, port, username, password)
SELECT lower(hex(randomblob(16))), ssh_user || '@' || ssh_host, ssh_host, ssh_port, ssh_user,
       max(CASE WHEN ssh_key_path = '' THEN ssh_password ELSE '' END)
FROM db_connections
WHERE tunnel = 'ssh'
GROUP BY ssh_host, ssh_port, ssh_user;

UPDATE db_connections
SET ssh_host_id = (
    SELECT ssh_hosts.id FROM ssh_hosts
    WHERE ssh_hosts.host = db_connections.ssh_host
      AND ssh_hosts.port = db_connections.ssh_port
      AND ssh_hosts.username = db_connections.ssh_user
)
WHERE tunnel = 'ssh';

INSERT INTO kubeconfigs (id, name, config)
SELECT lower(hex(randomblob(16))), min(name), kube_config
FROM db_connections
WHERE tunnel = 'kubernetes' AND kube_config <> ''
GROUP BY kube_config;

UPDATE db_connections
SET kubeconfig_id = (SELECT kubeconfigs.id FROM kubeconfigs WHERE kubeconfigs.config = db_connections.kube_config)
WHERE tunnel = 'kubernetes';

ALTER TABLE db_connections DROP COLUMN ssh_host;
ALTER TABLE db_connections DROP COLUMN ssh_port;
ALTER TABLE db_connections DROP COLUMN ssh_user;
ALTER TABLE db_connections DROP COLUMN ssh_password;
ALTER TABLE db_connections DROP COLUMN ssh_key_path;
ALTER TABLE db_connections DROP COLUMN kube_config;

CREATE INDEX db_connections_ssh_host_id_idx ON db_connections (ssh_host_id);
CREATE INDEX db_connections_kubeconfig_id_idx ON db_connections (kubeconfig_id);
