-- Which environment a connection reaches: 'local', 'development', 'staging'
-- or 'production'. It sets the connection's colour; groups only file it.
ALTER TABLE db_connections ADD COLUMN environment TEXT NOT NULL DEFAULT 'development';

-- Groups used to carry the colour, so a group named for an environment
-- passes that environment on.
UPDATE db_connections SET environment = CASE
    WHEN lower(g.name) LIKE '%prod%' THEN 'production'
    WHEN lower(g.name) LIKE '%stag%' THEN 'staging'
    WHEN lower(g.name) LIKE '%local%' THEN 'local'
    ELSE 'development'
END
FROM connection_groups AS g
WHERE g.id = db_connections.group_id;
