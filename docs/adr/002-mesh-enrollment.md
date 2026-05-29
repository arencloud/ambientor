# ADR 002: Mesh enrollment (istiod-aware, multi-mesh)

## Status

Accepted (implementation in progress)

## Context

- OSSM3 / SailOperator clusters may run **multiple istiod revisions** (e.g. `demo` and `ambient`), each with its own control-plane namespace.
- Namespace association with a control plane is **not** always `istio-discovery=<arbitrary>`:
  - Some installs use custom discovery label keys/values.
  - Some use **revision only** (`istio.io/rev`) with no discovery label.
  - OSSM also uses **`ServiceMeshMemberRoll`** membership.
- Ambientor must support **Web UI, CLI, and CRDs** for the same operations.

Example (cl01):

| Namespace       | Labels (mesh) |
|-----------------|---------------|
| `mesh-sidecar-2` | `istio-discovery=mesh-demo`, `istio.io/rev=demo` |
| `ambient-demo`   | `istio-discovery=mesh-ambient`, `istio.io/dataplane-mode=ambient` |

A rollout targeting the **ambient** mesh must enroll `mesh-sidecar-2` on that mesh before waypoint/policy stages.

## Decision

1. **`MeshEnrollment` on `MeshInstance`** — resolved per istiod revision from:
   - istiod ConfigMap `mesh` → `discoverySelectors` (authoritative when present)
   - enrolled namespace labels (observed)
   - OSSM MemberRoll presence in the control-plane namespace

2. **`MeshTarget` selection** — explicit `revision`, `discoveryLabel`, or `controlPlaneNamespace`; auto-select only when exactly one **ambient** instance exists.

3. **Rollout stage `EnrollNamespace`** — applies enrollment (MemberRoll + labels) before ambient labeling/waypoint. Planner order per wave:
   - `DryRun` → `EnrollNamespace` → `RemoveInjection` → `RollingRestart` → `LabelNamespace` → `DeployWaypoint` → `TranslatePolicy` → `RollingRestart` → `VerifyTraffic`

4. **Triple interface**
   - **CRD**: `Rollout` stages / `MigrationPlan` → `Rollout`
   - **API**: `GET /api/v1/mesh-instances`, `POST /api/v1/mesh-instances/enroll`
   - **CLI**: `ambientor mesh instances`, `ambientor mesh enroll --namespace …`

5. **Preflight** uses `MeshEnrollment`, not hardcoded `istio-discovery`. Dry-run allows namespaces that will be fixed by a later `EnrollNamespace` stage.

## Consequences

- Multiple ambient meshes require explicit `rollout.spec.meshTarget`.
- Enrollment without istiod `discoverySelectors` falls back to observed namespace labels or `mesh-{revision-short}`.
- Sidecar removal still requires `RemoveInjection` + `RollingRestart` stages (or manual equivalent).
