# Production pilot runbook

Use this runbook for **P1** (multi-cluster assessment validation) and **P2** (human-approved plans with exports). **P4** (portal OIDC approve) is covered in [roadmap/portal-oidc.md](roadmap/portal-oidc.md).

| Item | Value |
|------|--------|
| **Progress tracker** | [PROGRESS.md](PROGRESS.md) |
| **Lab baseline** | [runbook-lab.md](runbook-lab.md) |
| **OIDC API** | [roadmap/oidc-auth.md](roadmap/oidc-auth.md) |

---

## Prerequisites

| Tool | Notes |
|------|--------|
| `kubectl` | Access to each pilot cluster |
| `ambientor` CLI | Built from this repo or image `quay.io/arencloud/ambientor-cli:<tag>` |
| Portal + API | Helm release with optional `DATABASE_URL` and OIDC env for P4 |

Record cluster metadata in a spreadsheet or git-ignored folder: cluster name, Istio version, platform (GKE/EKS/OCP), date, operator image tag.

### One-command validation (P1 + P2)

From the repo root (VPN / cluster access required):

```bash
cp scripts/pilot-contexts.example scripts/pilot-contexts.txt
# Edit: one kubectl context per line (minimum 3 for P1)
cargo build -p ambientor-cli --release
./scripts/pilot-validate.sh
```

This writes `pilot-artifacts/<date>-validate/PILOT-SIGNOFF.md` and per-cluster JSON/SARIF/plan exports. Exit `0` when ≥3 clusters pass blocker analysis and ≥1 cluster exports a plan.

Optional: `PILOT_BLOCKER_ALLOWLIST=docs/pilot/allowlist-blockers.txt` for known platform-specific blockers.

---

## P1 — Blockers match Istio migrate docs (3+ clusters)

### Per cluster

1. Install or upgrade Ambientor (operator + API + web) to the pilot image tag.
2. Run assessment and capture evidence:

```bash
export KUBECONFIG=/path/to/cluster.kubeconfig
./scripts/pilot-record-assessment.sh ambientor-pilot \
  ./pilot-artifacts/$(date +%Y%m%d)-cluster-a
```

Or manually:

```bash
ambientor assess --output json > assessment.json
ambientor assess --output sarif > assessment.sarif
kubectl get ambientassessment -A
```

3. Compare **blocker** findings to [Istio ambient migration](https://istio.io/latest/docs/ambient/install/migrate/) prerequisites for that Istio minor version.
4. Mark P1 ✅ when three or more clusters show **no false-positive blockers** (document any expected platform-specific warnings in notes).

### Sign-off template

| Cluster | Istio | Blockers OK | SARIF path | Notes |
|---------|-------|-------------|------------|-------|
| | | ⬜ | | |

---

## P2 — Plans human-approved with exported manifests

### Portal workflow (hub + spokes)

1. **Hub cluster:** ensure `DATABASE_URL` is set on API and operator so findings and candidates persist in Postgres.
2. **Spoke clusters:** register `ClusterConnection` CRs on the hub (see [roadmap/hub-aggregation.md](roadmap/hub-aggregation.md)).
3. In the portal **Cluster** selector, choose a remote connection or fleet cluster, then **Run assessment** (remote connections use `POST /api/v1/connections/{ns}/{name}/assess`).
4. Review **Migration candidates** filtered by `clusterRef`; select namespaces and create a **Migration plan** (selection-based plan with `selectedNamespaces`).
5. Approve the plan in the portal or via `kubectl patch migrationplan … status.approved=true`.
6. Export the GitOps bundle (portal **Download YAML**, API export, or CLI).

**CLI shortcut (selection plan on hub):**

```bash
# After assessment CR exists (e.g. bookinfo-scan-assessment):
./scripts/pilot-ensure-selection-plan.sh <kubectl-context> <namespace> <assessment-name>
./scripts/pilot-export-plans.sh <kubectl-context> ./pilot-artifacts/my-run
```

### Per approved migration

1. Confirm assessment completed for the target cluster (`AmbientAssessment` phase `Completed` on hub, or Postgres scan row for remote assess).
2. Review operator-generated `MigrationPlan` in portal **Migration Plans** or:

```bash
kubectl get migrationplan -n <ns>
kubectl get migrationplan <name> -n <ns> -o yaml
```

3. Human sign-off (change ticket / email / PR comment) referencing plan name and assessment ref.
4. Export GitOps bundle:

   - Portal: **Download YAML bundle**, or
   - API: `GET /api/v1/plans/{namespace}/{name}/export`, or
   - CLI: `ambientor plan export -n <ns> <name> -o plan-bundle.yaml`

5. Store export in your config repo or artifact store; mark P2 ✅ when at least one plan per pilot environment is approved and exported.

---

## P4 — Portal approve with OIDC (quick check)

1. Install or upgrade with Postgres and auth (Helm example in [roadmap/helm-production.md](roadmap/helm-production.md)):

```bash
helm upgrade --install ambientor deploy/helm/ambientor/ \
  -n ambientor-system --create-namespace \
  --set auth.createSecret=true \
  --set auth.jwtSecret="$(openssl rand -base64 32)" \
  --set auth.oidc.enabled=true \
  --set auth.oidc.issuerUrl=https://YOUR_IDP/... \
  --set auth.oidc.clientId=ambientor \
  --set auth.oidc.redirectUri=https://YOUR_API/api/v1/auth/oidc/callback \
  --set auth.oidc.clientSecret="$OIDC_CLIENT_SECRET" \
  --set auth.oidc.successUrl=https://YOUR_PORTAL/ \
  --set openshift.apiUrl=https://YOUR_API
```

2. Register a user or use IdP login; assign Casbin role with `rollout:approve` on the rollout namespace.
3. Open portal → sign in (local or **Sign in with SSO**) → **Rollouts** → **Approve current stage** with a stage awaiting approval.
4. Confirm audit row shows JWT username (not `portal`) when `DATABASE_URL` is set.

---

## P3 / P5

- **P3:** Covered by kind e2e (`scripts/e2e-kind-ambient.sh`, CI `e2e-kind.yml`).
- **P5:** Approve/apply/rollback audit with Postgres — verify on rollout detail **Audit log** after P4 approve.
