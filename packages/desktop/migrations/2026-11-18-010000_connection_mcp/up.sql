-- Which MCP tools AI clients may use on this connection. All off keeps the
-- connection out of MCP entirely.
ALTER TABLE db_connections ADD COLUMN mcp_select BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE db_connections ADD COLUMN mcp_insert BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE db_connections ADD COLUMN mcp_update BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE db_connections ADD COLUMN mcp_delete BOOLEAN NOT NULL DEFAULT 0;
