-- The kubeconfig a 'kubernetes' tunnel signs in with, as YAML. The tunnel
-- kind was 'kubectl' while it ran the kubectl on PATH.
ALTER TABLE db_connections ADD COLUMN kube_config TEXT NOT NULL DEFAULT '';
UPDATE db_connections SET tunnel = 'kubernetes' WHERE tunnel = 'kubectl';
