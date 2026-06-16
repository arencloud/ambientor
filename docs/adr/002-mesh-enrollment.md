# ADR 002: Mesh enrollment (istiod-aware, multi-mesh)

## Status

Accepted — core delivered; OpenShift/Sail hardening ongoing

## Context

- OSSM3 / SailOperator clusters may run **multiple istiod revisions** (e.g. `demo` and `ambient`), each with its own control-plane namespace.
- Namespace association with a control plane is **not** always `istio-discovery=<arbitrary>`:
  - Some installs use custom discovery label keys/values.
  - Some use **revision only** (`istio.io/rev`) with no discovery label.
  - OSSM also uses **`ServiceMeshMemberRoll`** membership.
- Sail Operator exposes cluster-scoped **`IstioRevisionTag`** CRs (`sailoperator.io/v1`). When a tag exists, namespace `istio.io/rev` should use the **tag name** (e.g. `ambient`), not the underlying istiod revision string (e.g. `ambient-v1-28-6`).
- Ambientor must support **Web UI, CLI, and CRDs** for the same operations.

Example (cl01):

| Namespace       | Labels (mesh) |
|-----------------|---------------|
| `mesh-sidecar-2` | `istio-discovery=mesh-demo`, `istio.io/rev=demo` |
| `ambient-demo`   | `istio-discovery=mesh-ambient`, `istio.io/dataplane-mode=ambient` (no `istio.io/rev`) |

A rollout targeting the **ambient** mesh must enroll sidecar namespaces on that mesh before waypoint/policy stages.

## Decision

1. **`MeshEnrollment` on `MeshInstance`** — resolved per istiod revision from:
   - istiod ConfigMap `mesh` → `discoverySelectors` (authoritative when present)
   - enrolled namespace labels (observed)
   - OSSM MemberRoll presence in the control-plane namespace
   - Sail / upstream **`IstioRevisionTag`** → preferred `istio.io/rev` label value (`revision_tags.rs`)

2. **`MeshTarget` selection** — explicit `revision`, `discoveryLabel`, or `controlPlaneNamespace`; auto-select only when exactly one **ambient** instance exists.

3. **Rollout stage `EnrollNamespace`** — applies enrollment (MemberRoll + labels) before ambient labeling/waypoint. Planner order per wave (current):
   - `DryRun` → `EnrollNamespace` → `RemoveInjection` → `LabelNamespace` → `RollingRestart` → `DeployWaypoint` → `TranslatePolicy` → `RollingRestart` (`{wave}-restart-final`) → `VerifyTraffic`

   **Label / revision semantics (OpenShift ambient):**
   - `EnrollNamespace` sets discovery + `istio.io/rev=<revisionTag>` when a Sail tag exists.
   - `LabelNamespace` sets `istio.io/dataplane-mode=ambient` and clears `istio-injection`; it does **not** clear `istio.io/rev` yet.
   - `DeployWaypoint` waits for the Gateway to program, then clears `istio.io/rev` on ambient meshes so workloads do not retain sidecar injection while native ambient namespaces remain discovery + dataplane-mode only.
   - `VerifyTraffic` polls workload readiness (up to 180s) and rejects init-container sidecars (`istio-proxy`, `istio-validation`) and `sidecar.istio.io/status`.

4. **Triple interface**
   - **CRD**: `Rollout` stages / `MigrationPlan` → `Rollout` (includes `EnrollNamespace`)
   - **API**: `GET /api/v1/mesh-instances`, `POST /api/v1/mesh-instances/enroll`
   - **CLI**: `ambientor mesh instances`, `ambientor mesh enroll --namespace …`
   - **Portal**: mesh-instance list + enrollment label preview on Plans; standalone enroll is via rollout or API/CLI (no dedicated enroll button yet).

5. **Preflight** uses `MeshEnrollment`, not hardcoded `istio-discovery`. Dry-run allows namespaces that will be fixed by a later `EnrollNamespace` stage.

## Consequences

- Multiple ambient meshes require explicit `rollout.spec.meshTarget`.
- Enrollment without istiod `discoverySelectors` falls back to observed namespace labels or `mesh-{revision-short}`.
- Sidecar removal requires `RemoveInjection`, ambient labeling, and rolling restart; verify enforces no sidecar remnants.
- Clearing `istio.io/rev` before waypoint programming can leave Gateways stuck at **Waiting for controller** on multi-revision Sail clusters — rev is cleared only after the waypoint is programmed.

## Remaining gaps

- Read istiod `discoverySelectors` reliably on all Sail/OSSM control-plane ConfigMap layouts (`fromIstiodConfig` still false on cl01).
- Broader validation of `RevisionOnly` enrollment and full OSSM `ServiceMeshMemberRoll` mode beyond cl01.
- Portal action for `POST /api/v1/mesh-instances/enroll` (optional; rollout path covers migration).
- Update ADR stage list in dependent runbooks when hub remote rollout lands (ADR 003).

## Validation (cl01, 2026-06)

- `mesh-sidecar-1` and multi-namespace plan `migrate-mesh-sidecar-mesh-sidecar-20260616123221` reached rollout **Completed** after Sail tag resolution, stage reorder, waypoint rev timing, and verify hardening (`b1c4785`).
