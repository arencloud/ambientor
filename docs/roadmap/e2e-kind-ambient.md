# kind e2e: bookinfo → plan → rollout (Phase 3.5)

## Goal

Prove the migration path end-to-end on kind with Istio ambient and bookinfo.

## Delivered

- [x] `scripts/e2e-kind-ambient.sh` — cluster, Istio ambient, minimal bookinfo (`docs/lab/bookinfo-e2e.yaml`), Ambientor, inventory → assessment → plan → rollout (with stage approvals) → ambient namespace label
- [x] `deploy/helm/ambientor/values-e2e.yaml` — no Postgres, `web.replicas: 0` (faster CI)
- [x] Helm install without `--wait`; explicit `kubectl wait` on operator + API
- [x] Rollback behavior covered by `ambientor-rollout` unit tests; full rollback injection e2e deferred

## Branch

`cursor/e2e-kind-ambient`

## Local run

```bash
./scripts/e2e-kind-ambient.sh
# Reuse cluster: SKIP_CLUSTER_CREATE=1 ./scripts/e2e-kind-ambient.sh
```
