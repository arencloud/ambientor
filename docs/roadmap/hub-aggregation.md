# Hub ClusterConnection remote clients (Phase 4.3)

## Goal

Build kube clients from `ClusterConnection` credential secrets, verify remote API reachability, and expose hub APIs for listing connections and assessing remote clusters.

## Delivered

- [x] `ambientor-k8s::remote` — parse `kubeconfig` or bearer `token` (+ optional `ca.crt`, `server`) secrets
- [x] Hub operator reconciler — phases `SecretMissing`, `InvalidConfig`, `Unreachable`, `Connected` + Ready condition
- [x] `GET /api/v1/connections` — list `ClusterConnection` status from hub cluster
- [x] `POST /api/v1/connections/{namespace}/{name}/assess` — run assessment on remote cluster; persist scans with `cluster_ref` `{namespace}/{name}`
- [x] Sample manifest `config/samples/ambientor_v1alpha1_clusterconnection.yaml`

## Branch

`cursor/hub-aggregation`

## Credentials secret format

Either:

- `kubeconfig`: full kubeconfig YAML (optional `spec.apiServer` override), or
- `token` + `spec.apiServer` or secret key `server`, optional `ca.crt` / `ca-bundle`

## Notes

- `spec.hub: true` marks the local cluster entry; remote assess is rejected for hub connections.
- Spoke inventory/rollout on remote clusters is future work; this step validates clients and remote assess.
