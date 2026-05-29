-- Materialized dataplane mode per application assessment (ambient | sidecar | notEnrolled).

ALTER TABLE application_assessments
    ADD COLUMN IF NOT EXISTS dataplane_mode TEXT NOT NULL DEFAULT 'notEnrolled';

CREATE INDEX IF NOT EXISTS idx_application_assessments_dataplane
    ON application_assessments (cluster_id, dataplane_mode);
