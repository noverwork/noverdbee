-- The key's SHA-256 fingerprint, worked out when the key is saved; empty for
-- keys saved before this column existed.
ALTER TABLE ssh_keys ADD COLUMN fingerprint TEXT NOT NULL DEFAULT '';
