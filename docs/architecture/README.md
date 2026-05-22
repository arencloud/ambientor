# Ambientor architecture

See the repository [README](../../README.md) and CRDs under [`config/crd/`](../../config/crd/).

## Components

| Component | Role |
|-----------|------|
| `ambientor-operator` | Reconciles CRDs, runs scans, executes rollouts |
| `ambientor-api` | REST + SSE for portal and CLI |
| `ambientor-web` | Operator dashboard |
| `ambientor-cli` | `assess`, `plan`, `rollout` commands |

## Data flow

1. User creates `MeshInventory` with `triggerScan: true`
2. Operator creates `AmbientAssessment` and runs rules from `ambientor-scan` + `ambientor-analyze`
3. User reviews assessment in portal, creates `MigrationPlan`
4. User creates `Rollout` and approves stages in portal
5. Operator applies changes via `ambientor-rollout` engine

## Multi-cluster

Register remote clusters with `ClusterConnection` on the hub cluster. Credentials are stored in Kubernetes Secrets; the hub controller validates references and aggregates status.

## Implementation progress

Phased tasks, PR history, and **next step**: [PROGRESS.md](../PROGRESS.md).

Lab validation runbook: [runbook-lab.md](../runbook-lab.md).
