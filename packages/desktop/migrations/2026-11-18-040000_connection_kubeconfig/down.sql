UPDATE db_connections SET tunnel = 'kubectl' WHERE tunnel = 'kubernetes';
ALTER TABLE db_connections DROP COLUMN kube_config;
