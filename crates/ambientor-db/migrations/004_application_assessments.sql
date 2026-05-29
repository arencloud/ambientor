-- Per-application assessment materialization (portal list + detail, shared with dashboard).

CREATE TABLE IF NOT EXISTS assessment_runs (
    id UUID PRIMARY KEY,
    cluster_id UUID NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'completed',
    application_count INT NOT NULL DEFAULT 0,
    cluster_scores JSONB NOT NULL DEFAULT '{}',
    cluster_summary JSONB NOT NULL DEFAULT '{}',
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_assessment_runs_cluster_time
    ON assessment_runs (cluster_id, finished_at DESC);

CREATE TABLE IF NOT EXISTS application_assessments (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES assessment_runs(id) ON DELETE CASCADE,
    cluster_id UUID NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
    namespace TEXT NOT NULL,
    mesh_revision TEXT,
    discovery_label TEXT,
    control_plane_namespace TEXT,
    hostnames JSONB NOT NULL DEFAULT '[]',
    namespace_labels JSONB NOT NULL DEFAULT '{}',
    ingress_gateway_namespace TEXT,
    ingress_same_namespace BOOLEAN NOT NULL DEFAULT FALSE,
    workload_count INT NOT NULL DEFAULT 0,
    readiness_pct SMALLINT NOT NULL DEFAULT 0,
    risk_level TEXT NOT NULL DEFAULT 'low',
    blocker_count INT NOT NULL DEFAULT 0,
    warning_count INT NOT NULL DEFAULT 0,
    scores JSONB NOT NULL DEFAULT '{}',
    summary JSONB NOT NULL DEFAULT '{}',
    findings JSONB NOT NULL DEFAULT '[]',
    suggestions JSONB NOT NULL DEFAULT '[]',
    assessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, namespace),
    CONSTRAINT application_assessments_risk CHECK (
        risk_level IN ('low', 'medium', 'high', 'critical')
    )
);

CREATE INDEX IF NOT EXISTS idx_application_assessments_run
    ON application_assessments (run_id);

CREATE INDEX IF NOT EXISTS idx_application_assessments_cluster_ns
    ON application_assessments (cluster_id, namespace);

CREATE INDEX IF NOT EXISTS idx_application_assessments_risk
    ON application_assessments (cluster_id, risk_level);

CREATE INDEX IF NOT EXISTS idx_application_assessments_readiness
    ON application_assessments (cluster_id, readiness_pct);
