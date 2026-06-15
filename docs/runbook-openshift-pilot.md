# OpenShift pilot test runbook

Use this runbook to validate Ambientor on **real OpenShift/OSSM clusters** after the Tier 1–4 changes (Postgres findings, multicluster portal, rollback, Helm OIDC).

| Item | Value |
|------|--------|
| **Progress** | [PROGRESS.md](PROGRESS.md) |
| **Helm values** | [deploy/helm/ambientor/values-openshift-pilot.yaml](../deploy/helm/ambientor/values-openshift-pilot.yaml) |
| **Generic pilot** | [runbook-pilot.md](runbook-pilot.md) |

---

## Prerequisites

| Tool | Notes |
|------|--------|
| `oc` | Logged in to hub cluster with `cluster-admin` or sufficient RBAC |
| `helm` 3 | |
| `curl`, `jq` | Smoke script |
| Quay pull | `quay.io/arencloud/ambientor-{operator,api,web}:0.1.4` (or your tag) |
| Storage class | PVC for Postgres (set `AMBIENTOR_STORAGE_CLASS` on install) |

---

## 1 — Hub install

```bash
export AMBIENTOR_STORAGE_CLASS=gp3   # your cluster default / CSI class
chmod +x scripts/openshift-pilot-install.sh scripts/openshift-pilot-smoke.sh
./scripts/openshift-pilot-install.sh
./scripts/openshift-pilot-smoke.sh
```

Optional OIDC (set before install):

```bash
export AMBIENTOR_OIDC_ENABLED=1
export AMBIENTOR_OIDC_ISSUER_URL=https://idp.example.com/realms/ambientor
export AMBIENTOR_OIDC_CLIENT_ID=ambientor
export AMBIENTOR_OIDC_CLIENT_SECRET=...
# After install, script prints redirect URI; or set:
# export AMBIENTOR_OIDC_REDIRECT_URI=https://<api-route>/api/v1/auth/oidc/callback
./scripts/openshift-pilot-install.sh
```

Record URLs from install output in your pilot notes.

---

## 2 — Assessment + Postgres findings (Tier 1)

| Step | Action | Pass |
|------|--------|------|
| 2.1 | Portal → **Run assessment** (hub) | Candidates appear in **Migration candidates** |
| 2.2 | `oc get ambientassessment -n ambientor-system` phase `Completed` | |
| 2.3 | `curl -s $API/api/v1/assessments \| jq '.[0].findings \| length'` | > 0 when Postgres enabled |
| 2.4 | CLI: `ambientor assess --output json` | Blockers match Istio migrate docs (P1) |

---

## 3 — Multicluster spoke (Tier 2)

| Step | Action | Pass |
|------|--------|------|
| 3.1 | On spoke: create credentials `Secret` (kubeconfig or token) | |
| 3.2 | Hub: apply [docs/lab/clusterconnection-spoke.example.yaml](lab/clusterconnection-spoke.example.yaml) | `ClusterConnection` phase `Connected` |
| 3.3 | Portal **Cluster** picker → select spoke → **Run assessment** | Candidates with spoke `clusterRef` |
| 3.4 | Dashboard **Cluster connections** panel shows spoke | |
| 3.5 | `curl -s $API/api/v1/dashboard/fleet \| jq '.clusters \| length'` | ≥ 2 after spoke sync |

---

## 4 — P2 plan + export

```bash
# After hub or spoke assess:
./scripts/pilot-ensure-selection-plan.sh <hub-context> <namespace> <assessment-name>
./scripts/pilot-export-plans.sh <hub-context> ./pilot-artifacts/openshift-$(date +%Y%m%d)
```

| Step | Action | Pass |
|------|--------|------|
| 4.1 | Portal → select namespaces → **Create plan** | `MigrationPlan` phase `Ready` |
| 4.2 | Human approve (ticket / PR comment) | |
| 4.3 | Portal **Download YAML** or CLI export | Bundle in artifact store |

---

## 5 — Rollout + rollback (Tier 3)

| Step | Action | Pass |
|------|--------|------|
| 5.1 | Portal → plan → **Start rollout** → approve once | Pipeline runs automatically |
| 5.2 | Rollout phase `Completed`; namespace `istio.io/dataplane-mode=ambient` | |
| 5.3 | (Optional) Repeat kind-style rollback: fail verify, expect `RolledBack` | See `scripts/e2e-kind-ambient.sh` rollback section |

On OpenShift/OSSM, confirm **EnrollNamespace** / MemberRoll if namespace was not pre-enrolled.

```bash
ambientor openshift wizard --enroll --namespace <ns>
```

---

## 6 — P4 OIDC approve (optional)

1. Install with OIDC env vars (§1).
2. Portal → **Sign in with SSO** → **Rollouts** → **Approve**.
3. Rollout audit shows IdP username (not `portal`).

---

## 7 — Full pilot validate (multi-cluster)

```bash
cp scripts/pilot-contexts.example scripts/pilot-contexts.txt
# Edit contexts (hub + spokes)
./scripts/pilot-validate.sh
```

---

## Sign-off checklist

| Area | Status | Notes |
|------|--------|-------|
| Hub install + smoke | ⬜ | |
| P1 assessments (3+ clusters) | ⬜ | |
| Spoke connection + remote assess | ⬜ | |
| P2 plan export | ⬜ | |
| Rollout happy path | ⬜ | |
| Rollback (optional) | ⬜ | |
| OIDC approve (optional) | ⬜ | |

---

## Troubleshooting

| Symptom | Check |
|---------|--------|
| API pod not ready | `oc logs -n ambientor-system deployment/<release>-ambientor-api` |
| Empty findings in API | `DATABASE_URL` on operator + API; `scan_runs` in Postgres |
| Portal cannot reach API | `config.js` → `AMBIENTOR_API_URL` matches API Route; CORS not required (same-origin via Route) |
| Postgres pending | Storage class, SCC, RHEL postgres image pull |
| Spoke assess fails | `ClusterConnection` Ready; hub can reach spoke API |
