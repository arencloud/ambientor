# Platform CI — kind and OpenShift (Phase 5.2)

## Goal

Run automated platform checks in GitHub Actions: kind e2e for the migration path, and OpenShift-focused smoke tests without requiring a live OSSM cluster in CI.

## Delivered

- [x] **kind e2e** — `.github/workflows/e2e-kind.yml` on `main` / PRs (Istio ambient + bookinfo → plan → rollout)
- [x] **OpenShift smoke** — `.github/workflows/openshift-smoke.yml` (wizard unit tests + `helm lint`)
- [x] **Image release** — `.github/workflows/images.yml` on version tags `v*` only (not every `main` push)

## Branch

Merged via PR [#25](https://github.com/arencloud/ambientor/pull/25).

## CI workflows

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `ci.yml` | `main`, PR | fmt, clippy, test, cargo-deny |
| `e2e-kind.yml` | `main`, PR | Full kind + Istio ambient e2e |
| `openshift-smoke.yml` | `main`, PR | OSSM wizard tests, Helm lint |
| `images.yml` | `v*` tag, manual | Push multi-arch images to Quay |

## Release images

```bash
git tag v0.1.0
git push origin v0.1.0
```

Publishes `quay.io/arencloud/ambientor-{operator,api,web,cli}:0.1.0`. Manual run: Actions → **Publish images** → **Run workflow** (optional version input).

## OpenShift lab (manual)

Full OSSM validation still requires a cluster — see `docs/runbook-lab.md` §13 and `ambientor openshift wizard` on OpenShift.
