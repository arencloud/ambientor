# Ambientor

**Ambient Mesh Migration Assistant** — analyze, plan, and execute safe migrations from Istio sidecar mode to Ambient mode on Kubernetes and OpenShift Service Mesh.

![Ambientor logo](docs/images/logo/ambientor.png)

## Features

- **Ambient readiness scanner** — workload and namespace compatibility scores with blockers and remediation hints
- **Sidecar dependency detector** — init chains, localhost proxy calls, injection assumptions
- **Traffic compatibility analyzer** — VirtualService/HTTPRoute conflicts, waypoint requirements, mixed-mode L7 gaps
- **Migration planner** — ordered waves, policy translation checklist, rollback points
- **Staged rollout engine** — approval-gated automated apply with verification and auto-rollback

## Architecture

In-cluster deployment via Helm: operator (controllers + scanners), API (REST/SSE), web portal, and Postgres. A shared Rust core powers the CLI and API.

```
ambientor-cli / ambientor-web → ambientor-api → ambientor-core
ambientor-operator → ambientor-scan / ambientor-analyze / ambientor-plan / ambientor-rollout
```

## Requirements

- Rust **1.95.0** (`rust-toolchain.toml`)
- Kubernetes 1.28+ or OpenShift 4.19+
- Istio 1.24+ (ambient) or OpenShift Service Mesh 3.2+

## Quick start

```bash
# Build
cargo build --release

# Install CRDs and Helm chart (cluster admin)
kubectl apply -k config/crd/
helm install ambientor deploy/helm/ambientor/ -n ambientor-system --create-namespace

# CLI (local or in-cluster API)
cargo run -p ambientor-cli -- assess --namespace bookinfo
```

## Lab validation

Step-by-step kind + Istio ambient + Bookinfo validation: [docs/runbook-lab.md](docs/runbook-lab.md).

**Progress tracker** (phases, steps, what is done / next): [docs/PROGRESS.md](docs/PROGRESS.md).

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

Apache License 2.0 — see [LICENSE](LICENSE).
