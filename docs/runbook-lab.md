# Lab runbook — validate Ambientor on kind + Istio ambient

This runbook completes **Phase 1, Step 1.1**: prove the designed loop on a real mesh before further code changes.

| Item | Value |
|------|--------|
| **Designed flow** | `MeshInventory` → `AmbientAssessment` → (later) `MigrationPlan` → `Rollout` |
| **Progress tracker** | [PROGRESS.md](PROGRESS.md) |
| **Sample manifests** | [lab/](lab/) |
| **Automation** | [scripts/lab-build-images.sh](../scripts/lab-build-images.sh), [scripts/lab-kind-up.sh](../scripts/lab-kind-up.sh) |

---

## Prerequisites

| Tool | Version | Check |
|------|---------|--------|
| Docker | 24+ | `docker info` |
| kind | 0.22+ | `kind version` |
| kubectl | 1.28+ | `kubectl version --client` |
| Helm | 3.12+ | `helm version` |
| istioctl | 1.24+ (ambient-capable) | `istioctl version` |
| Rust | 1.95 | `rustc --version` (for local CLI) |

**Cluster resources (minimum):** 4 CPU, 8 GiB RAM for kind node.

---

## 1. Create kind cluster

```bash
./scripts/lab-kind-up.sh
# Or manually:
kind create cluster --name ambientor-lab --config docs/lab/kind-config.yaml
```

Verify:

```bash
kubectl cluster-info --context kind-ambientor-lab
```

---

## 2. Install Istio with ambient profile

```bash
istioctl install --set profile=ambient -y --context kind-ambientor-lab
kubectl get pods -n istio-system --context kind-ambientor-lab
```

**Expect:** `istiod`, `ztunnel`, and CNI pods Running.

### Gateway API CRDs (required for readiness rule)

```bash
kubectl apply -f https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.2.0/standard-install.yaml
kubectl get crd httproutes.gateway.networking.k8s.io
```

---

## 3. Deploy Bookinfo (sidecar mode)

```bash
kubectl create namespace bookinfo --context kind-ambientor-lab
kubectl label namespace bookinfo istio-injection=enabled --context kind-ambientor-lab
kubectl apply -n bookinfo -f https://raw.githubusercontent.com/istio/istio/release-1.24/samples/bookinfo/platform/kube/bookinfo.yaml --context kind-ambientor-lab
kubectl wait -n bookinfo --for=condition=ready pod -l app=ratings --timeout=300s --context kind-ambientor-lab
```

**Expect:** Each app pod has `istio-proxy` container (2/2 Ready).

```bash
kubectl get pods -n bookinfo -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.spec.containers[*].name}{"\n"}{end}'
```

---

## 4. Install Ambientor CRDs

From repository root:

```bash
kubectl apply -k config/crd/ --context kind-ambientor-lab
kubectl get crd | grep ambientor.io
```

**Expect:** `meshinventories`, `ambientassessments`, `migrationplans`, `rollouts`, etc.

---

## 5. Build and load container images

Published images may not exist on GHCR yet; build locally:

```bash
./scripts/lab-build-images.sh
./scripts/lab-load-kind.sh ambientor-lab
```

Images loaded:

- `ambientor:0.1.0-operator`
- `ambientor:0.1.0-api`
- `ambientor:0.1.0-web`

---

## 6. Install Ambientor Helm chart (lab values)

```bash
helm dependency update deploy/helm/ambientor/
helm upgrade --install ambientor deploy/helm/ambientor/ \
  -n ambientor-system --create-namespace \
  -f deploy/helm/ambientor/values-lab.yaml \
  --kube-context kind-ambientor-lab \
  --wait --timeout 10m
```

Verify:

```bash
kubectl get pods -n ambientor-system --context kind-ambientor-lab
kubectl logs -n ambientor-system -l app=ambientor-operator --tail=50 --context kind-ambientor-lab
```

**Expect:** operator, api, web, postgresql pods Running.

---

## 7. Register in-cluster cluster (optional)

```bash
kubectl apply -f config/samples/ambientor_v1alpha1_cluster.yaml --context kind-ambientor-lab
```

---

## 8. Trigger mesh inventory scan

```bash
kubectl apply -f docs/lab/meshinventory-bookinfo.yaml --context kind-ambientor-lab
```

Within a few seconds (operator watch reconcile), the operator should:

1. Patch `MeshInventory` status with `assessmentRef` and `observedGeneration`
2. Create or update a stable `AmbientAssessment` named `{inventory-name}-assessment` in namespace `ambientor-system`

Check:

```bash
kubectl get meshinventory -n ambientor-system --context kind-ambientor-lab -o yaml
kubectl get ambientassessment -n ambientor-system --context kind-ambientor-lab
```

Wait for assessment `Completed`:

```bash
kubectl wait -n ambientor-system --for=jsonpath='{.status.phase}'=Completed \
  ambientassessment --all --timeout=120s --context kind-ambientor-lab
```

---

## 9. Inspect assessment results

### kubectl

```bash
kubectl get ambientassessment -n ambientor-system -o jsonpath='{.items[0].status}' --context kind-ambientor-lab | jq .
```

Review:

| Field | What to check |
|-------|----------------|
| `overallScore` | 0–100 (lower if blockers) |
| `summary.blockers` | PeerAuth DISABLE, hold-until-proxy, VM workloads, etc. |
| `findings[].evidence` | Populated for policy/sidecar rules (PR #3) |
| `findings[].id` | e.g. `readiness.gateway-api`, `sidecar.localhost-proxy` |

### CLI (direct cluster, no API)

```bash
cargo run -p ambientor-cli -- assess --output json 2>/dev/null | jq '.summary, .findings[:5]'
```

Set context: `export KUBECONFIG=...` or `--kubeconfig` if kind merges into default context.

### Portal (optional)

```bash
kubectl port-forward -n ambientor-system svc/ambientor-web 3000:3000 --context kind-ambientor-lab
# Open http://localhost:3000 — dashboard is MVP; prefer kubectl/CLI for full findings today
```

---

## 10. Validation checklist (mark in lab notes)

Copy this table into your lab notes or PR comment when validating:

| # | Check | Pass? | Notes |
|---|--------|-------|-------|
| V1 | ztunnel / ambient data plane detected (`readiness.ambient-components` absent or info only) | | |
| V2 | Gateway API CRD present (no `readiness.gateway-api` warning) | | |
| V3 | VirtualServices listed in inventory (bookinfo has VS) | | |
| V4 | If mixed HTTPRoute + VS in cluster, `traffic.vs-httproute-conflict` fires | | N/A on stock bookinfo |
| V5 | Injected namespaces scanned for workloads | | |
| V6 | `MeshInventory` → `AmbientAssessment` linkage (`status.assessmentRef`) | | |
| V7 | Operator logs show no reconcile errors | | |

---

## 11. Known gaps (MVP — document during lab)

Record anything that fails or surprises you; these are **expected** follow-ups (see [PROGRESS.md](PROGRESS.md)):

| Gap | Workaround | Planned step |
|-----|------------|--------------|
| Re-scan requires spec change | Bump `MeshInventory` metadata generation (e.g. edit spec) while `triggerScan: true` | Documented behavior |
| Portal shows limited findings / no evidence UI | Use CLI `assess --output json` | 1.7 portal |
| Rollout rollback does not revert manifests yet | Test rollback carefully | Phase 3.2 |
| No `MigrationPlan` controller | Use CLI `plan` locally | 2.1 |
| GHCR images may be missing | Use `lab-build-images.sh` | 5.1 publish |
| OSSM MemberRoll not fully detected | Upstream Istio lab only in this runbook | 1.6 |
| Cluster-wide pod list (not namespaceSelector) | Full cluster scan | 1.6 |

---

## 12. Teardown

```bash
./scripts/lab-kind-down.sh
# Or: kind delete cluster --name ambientor-lab
```

---

## 13. OpenShift / OSSM lab (outline)

For OSSM 3.2+ on OpenShift, reuse steps 4–9 after:

1. Install OSSM operator and `ServiceMeshControlPlane` with ambient profile per Red Hat docs.
2. Label target namespace for member roll / injection per OSSM guidance.
3. Set `MeshInventory.spec.clusterRef` to your `Cluster` CR name.

Document OSSM-specific gaps in [PROGRESS.md](PROGRESS.md) step 1.6.

---

## Quick reference — file map

| Path | Purpose |
|------|---------|
| `docs/lab/kind-config.yaml` | kind cluster config |
| `docs/lab/meshinventory-bookinfo.yaml` | Trigger scan |
| `deploy/helm/ambientor/values-lab.yaml` | Local images, smaller Postgres |
| `scripts/lab-*.sh` | Build, load, up, down |

When Step 1 is validated on your machine, update [PROGRESS.md](PROGRESS.md) step **1.1** notes with date and cluster type (e.g. `kind+istio-1.24-ambient`).
