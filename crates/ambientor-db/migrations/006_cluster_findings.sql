-- Cluster-scoped findings from the latest assessment run (not tied to one namespace).

ALTER TABLE assessment_runs
    ADD COLUMN IF NOT EXISTS cluster_findings JSONB NOT NULL DEFAULT '[]';
