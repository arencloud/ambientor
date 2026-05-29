-- Multicluster dashboard materialization (ADR 003).
-- Sync from operator/API is a follow-up; tables are ready for hub aggregation.

CREATE TABLE IF NOT EXISTS clusters (
    id UUID PRIMARY KEY,
    cluster_ref TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    platform TEXT,
    mesh_flavor TEXT,
    istio_version TEXT,
    api_server_url TEXT,
    is_hub BOOLEAN NOT NULL DEFAULT FALSE,
    connection_namespace TEXT,
    connection_name TEXT,
    reachable BOOLEAN,
    last_seen_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT clusters_connection_pair CHECK (
        (connection_namespace IS NULL AND connection_name IS NULL)
        OR (connection_namespace IS NOT NULL AND connection_name IS NOT NULL)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_clusters_connection
    ON clusters (connection_namespace, connection_name)
    WHERE connection_namespace IS NOT NULL;

CREATE TABLE IF NOT EXISTS mesh_instances (
    id UUID PRIMARY KEY,
    cluster_id UUID NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    discovery_label TEXT NOT NULL,
    control_plane_namespace TEXT NOT NULL,
    version TEXT,
    ambient BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (cluster_id, revision, discovery_label, control_plane_namespace)
);

CREATE INDEX IF NOT EXISTS idx_mesh_instances_cluster ON mesh_instances(cluster_id);

CREATE TABLE IF NOT EXISTS application_status (
    id UUID PRIMARY KEY,
    cluster_id UUID NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
    mesh_instance_id UUID REFERENCES mesh_instances(id) ON DELETE SET NULL,
    namespace TEXT NOT NULL,
    status TEXT NOT NULL,
    ambient_dataplane BOOLEAN NOT NULL DEFAULT FALSE,
    blocker_count INT NOT NULL DEFAULT 0,
    rollout_phase TEXT,
    assessment_ref TEXT,
    mesh_revision TEXT,
    discovery_label TEXT,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (cluster_id, namespace),
    CONSTRAINT application_status_valid CHECK (
        status IN (
            'migrated',
            'processing',
            'blocker',
            'failed',
            'scanned',
            'not_scanned'
        )
    )
);

CREATE INDEX IF NOT EXISTS idx_application_status_cluster_status
    ON application_status (cluster_id, status);

CREATE INDEX IF NOT EXISTS idx_application_status_mesh
    ON application_status (mesh_instance_id);

CREATE TABLE IF NOT EXISTS dashboard_snapshots (
    id UUID PRIMARY KEY,
    cluster_id UUID NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
    summary JSONB NOT NULL DEFAULT '{}',
    mesh_instances JSONB NOT NULL DEFAULT '[]',
    captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dashboard_snapshots_cluster_time
    ON dashboard_snapshots (cluster_id, captured_at DESC);

-- Link historical scans to cluster registry when present.
ALTER TABLE scan_runs
    ADD COLUMN IF NOT EXISTS cluster_id UUID REFERENCES clusters(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_scan_runs_cluster_id ON scan_runs(cluster_id);
