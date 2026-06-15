# Ambientor — architecture progress tracker

Use this file to see **what is done**, **what is in progress**, and **what to do next**.  
Agents should update status when a step is started, merged, or blocked.

**Legend:** ✅ done · 🔄 in progress · ⬜ pending · ⏸ blocked

**Current focus:** Tier 3 rollout safety — enroll/injection rollback complete; rollback failure e2e pending.

**Next up:** Extend `e2e-kind-ambient.sh` with rollback failure injection, then P2 pilot sign-off on a spoke.

**Last updated:** 2026-05-28

---

## Designed flow (reference)

```mermaid
flowchart LR
  MI[MeshInventory] --> AA[AmbientAssessment]
  AA --> MP[MigrationPlan]
  MP --> RO[Rollout]
```

See [architecture/README.md](architecture/README.md) and [ADR 001](adr/001-in-cluster-deployment.md).

---

## Phase 0 — Foundation

| Step | Task | Status | Branch / PR | Notes |
|------|------|--------|-------------|-------|
| 0.1 | Rust 1.95 workspace, 14 crates, CRDs | ✅ | PR [#1](https://github.com/arencloud/ambientor/pull/1) | Merged |
| 0.2 | Helm chart, RBAC, operator + API + web | ✅ | PR #1 | Postgres optional via `DATABASE_URL` |
| 0.3 | CI: fmt, clippy, test, cargo-deny | ✅ | PR #1 | |
| 0.4 | Git rules (local author only; no co-author trailers) | ✅ | `scripts/git-hooks/` | |

---

## Phase 1 — Read path (trustworthy assessment)

| Step | Task | Status | Branch / PR | Notes |
|------|------|--------|-------------|-------|
| 1.1 | **Lab validation runbook** | ✅ | `docs/runbook-lab.md`, `docs/lab/*`, `scripts/lab-*` | Step 1 deliverable; you run on kind/lab |
| 1.2 | Real mesh inventory (Istio/Gateway API CRDs) | ✅ | PR [#2](https://github.com/arencloud/ambientor/pull/2) | `PolicyContext`, istiod version |
| 1.3 | Assessment evidence + sidecar/DR rules | ✅ | PR [#3](https://github.com/arencloud/ambientor/pull/3) | `Finding.evidence`, workload scan |
| 1.4 | Operator informers (replace 30s polling) | ✅ | PR [#5](https://github.com/arencloud/ambientor/pull/5) | kube-runtime watches; stable `{name}-assessment`; `observedGeneration` |
| 1.5 | Deeper rules (SPIRE, EF-on-waypoint, version gates) | ✅ | PR [#6](https://github.com/arencloud/ambientor/pull/6) | `PlatformContext`, Istio 1.24+ gate |
| 1.6 | OSSM namespace / MemberRoll inventory | ✅ | Part of 1.5 | MemberRoll list + enrollment warning |
| 1.7 | Portal assessment UI + evidence | ✅ | PR [#7](https://github.com/arencloud/ambientor/pull/7) | Merged |
| 1.8 | SARIF export (`ambientor assess --output sarif`) | ✅ | PR [#8](https://github.com/arencloud/ambientor/pull/8) | Merged |
| 1.9 | Persist scans in Postgres | ✅ | PR [#9](https://github.com/arencloud/ambientor/pull/9) | Merged; `GET /api/v1/scans` |

**Phase 1 exit criteria:** ✅ Assessment matches Istio migrate docs on real clusters; portal or SARIF shows evidence; operator uses watches.

---

## Phase 2 — Plan path

| Step | Task | Status | Branch / PR | Notes |
|------|------|--------|-------------|-------|
| 2.1 | `MigrationPlan` controller (assessment → plan CR) | ✅ | PR [#10](https://github.com/arencloud/ambientor/pull/10) | Merged |
| 2.2 | `PolicyTranslation` (VS → HTTPRoute suggestions) | ✅ | PR [#11](https://github.com/arencloud/ambientor/pull/11) | Merged |
| 2.3 | Portal plan review + manifest export | ✅ | PR [#12](https://github.com/arencloud/ambientor/pull/12) | Merged |
| 2.4 | CLI `plan create` + GitOps export | ✅ | PR [#13](https://github.com/arencloud/ambientor/pull/13) | Merged |

**Phase 2 exit criteria:** ✅ Human-approved plan with exported YAML/JSON; no rollout required (portal + CLI export).

---

## Phase 3 — Rollout path (approval-gated)

| Step | Task | Status | Branch / PR | Notes |
|------|------|--------|-------------|-------|
| 3.1 | Real `DeployWaypoint` / `TranslatePolicy` / restart / verify | ✅ | PR [#15](https://github.com/arencloud/ambientor/pull/15) | Merged |
| 3.2 | Rollback reverts labels/manifests | ✅ | PR [#16](https://github.com/arencloud/ambientor/pull/16) | Merged |
| 3.3 | Approval API + portal UI | ✅ | PR [#17](https://github.com/arencloud/ambientor/pull/17) | Merged |
| 3.4 | Audit log for approve/apply/rollback | ✅ | PR [#18](https://github.com/arencloud/ambientor/pull/18) | Merged |
| 3.5 | kind e2e: bookinfo → plan → rollout → verify | ✅ | PR [#19](https://github.com/arencloud/ambientor/pull/19) | `scripts/e2e-kind-ambient.sh`, `.github/workflows/e2e-kind.yml` |

**Phase 3 exit criteria:** ✅ One namespace at a time; verify + auto-rollback proven in e2e (happy-path kind job + rollback unit tests).

---

## Phase 4 — Enterprise

| Step | Task | Status | Branch / PR | Notes |
|------|------|--------|-------------|-------|
| 4.1 | Full OIDC (discovery + callback) | ✅ | PR [#20](https://github.com/arencloud/ambientor/pull/20) | Discovery + PKCE + API routes; see `docs/roadmap/oidc-auth.md` |
| 4.2 | Namespace-scoped Casbin in Postgres | ✅ | PR [#21](https://github.com/arencloud/ambientor/pull/21) | Postgres `casbin_rule` + domain model; approve gated; see `docs/roadmap/rbac-postgres.md` |
| 4.3 | Hub `ClusterConnection` remote clients | ✅ | PR [#22](https://github.com/arencloud/ambientor/pull/22) | Remote kube clients + assess API; portal cluster picker + connections panel |
| 4.4 | OpenShift OLM / SCC / MemberRoll wizard | ✅ | PR [#23](https://github.com/arencloud/ambientor/pull/23) | OLM + SCC + MemberRoll wizard; see `docs/roadmap/openshift-olm.md` |

---

## Phase 5 — Ecosystem

| Step | Task | Status | Notes |
|------|------|--------|-------|
| 5.1 | Publish Quay images (multi-arch) | ✅ | PR [#24](https://github.com/arencloud/ambientor/pull/24) | Tag `v*` → Quay; see `docs/roadmap/quay-images.md` |
| 5.2 | kind/OpenShift in CI | ✅ | PR [#25](https://github.com/arencloud/ambientor/pull/25) | `e2e-kind.yml` + `openshift-smoke.yml`; see `docs/roadmap/platform-ci.md` |
| 5.3 | Performance (10k pods / informer cache) | ✅ | PR [#26](https://github.com/arencloud/ambientor/pull/26) | `ClusterResourceCache`; see `docs/roadmap/performance-informer-cache.md` |
| 5.4 | Pluggable DB trait | ✅ | PR [#27](https://github.com/arencloud/ambientor/pull/27) | `ScanStore` / `AuditStore` / `UserStore`; see `docs/roadmap/pluggable-db.md` |
| 5.5 | Logo variants for UI | ✅ | PR [#28](https://github.com/arencloud/ambientor/pull/28) | Portal icon + favicon; see `docs/roadmap/logo-variants.md` |

---

## Production pilot checklist

| # | Criterion | Status |
|---|-----------|--------|
| P1 | Blockers match Istio migrate docs on 3+ clusters | ✅ `pilot-artifacts/20260527-validate` (cl01/cl02/cl03, 0 blockers) |
| P2 | Plans human-approved with exported manifests | 🔄 | Portal multicluster assess + Postgres findings wired; export on pilot pending |
| P3 | Rollout: one NS, verify + auto-rollback in e2e | ✅ |
| P4 | Portal/OIDC gates approve + execute | ✅ PR [#29](https://github.com/arencloud/ambientor/pull/29) |
| P5 | Audit log for approve / apply / rollback | ✅ |

---

## How to update this file

1. When starting work: set step to 🔄 and add branch name.
2. When PR merges: set ✅, add PR link, set **Next up** at top.
3. When blocked: set ⏸ and add one-line reason under Notes.

Roadmap detail per feature: `docs/roadmap/*.md`.
