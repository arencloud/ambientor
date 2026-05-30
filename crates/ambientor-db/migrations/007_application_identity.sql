-- Pod-derived application identity and migration candidacy for the portal list.

ALTER TABLE application_assessments
    ADD COLUMN IF NOT EXISTS application_name TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS workload_components JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS migration_candidate BOOLEAN NOT NULL DEFAULT TRUE;

CREATE INDEX IF NOT EXISTS idx_application_assessments_migration_candidate
    ON application_assessments (cluster_id, migration_candidate);
