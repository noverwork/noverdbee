-- SQL the user ran in an editor, newest last, per saved connection.
CREATE TABLE query_history (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL REFERENCES db_connections (id) ON DELETE CASCADE,
    sql TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX query_history_connection_id_idx ON query_history (connection_id);
