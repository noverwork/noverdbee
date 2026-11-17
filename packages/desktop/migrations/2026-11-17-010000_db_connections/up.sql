-- Groups now hold database connections, not hosts.
ALTER TABLE host_groups RENAME TO connection_groups;

-- A saved database connection. `driver` is 'sqlite' or 'postgres'; SQLite uses
-- `path` and `read_only`, PostgreSQL uses the server columns.
CREATE TABLE db_connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    driver TEXT NOT NULL,
    group_id TEXT REFERENCES connection_groups (id) ON DELETE SET NULL,
    host TEXT NOT NULL DEFAULT '',
    port INTEGER NOT NULL DEFAULT 0,
    database TEXT NOT NULL DEFAULT '',
    username TEXT NOT NULL DEFAULT '',
    password TEXT NOT NULL DEFAULT '',
    ssl_mode TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL DEFAULT '',
    read_only BOOLEAN NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX db_connections_group_id_idx ON db_connections (group_id);
