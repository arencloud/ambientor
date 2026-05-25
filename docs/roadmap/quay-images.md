# Quay multi-arch images (Phase 5.1)

## Goal

Publish operator, API, web, and CLI container images to Quay for `linux/amd64` and `linux/arm64`, matching Helm image references.

## Delivered

- [x] `.github/workflows/images.yml` — buildx push on `v*` tags and `workflow_dispatch` (not on every `main` push)
- [x] `.dockerignore` — smaller build context
- [x] One Quay repository per component

## Branch

Merged via PR [#24](https://github.com/arencloud/ambientor/pull/24); registry on Quay under `arencloud`.

## CI secrets

Add repository secrets for push access to `quay.io/arencloud`:

| Secret | Value |
|--------|--------|
| `QUAY_USERNAME` | Quay user or robot account name |
| `QUAY_PASSWORD` | Quay password or robot token |

Create a robot account under the `arencloud` organization with **Write** on each image repository (or org-wide).

## Quay repositories

Create these public (or pull-secret) repos under **arencloud**:

| Repository | Image |
|------------|--------|
| `ambientor-operator` | `quay.io/arencloud/ambientor-operator:<version>` |
| `ambientor-api` | `quay.io/arencloud/ambientor-api:<version>` |
| `ambientor-web` | `quay.io/arencloud/ambientor-web:<version>` |
| `ambientor-cli` | `quay.io/arencloud/ambientor-cli:<version>` (optional; not used by Helm) |

Example: `quay.io/arencloud/ambientor-operator:0.1.0`

## Release

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Helm

`deploy/helm/ambientor/values.yaml`:

```yaml
image:
  registry: quay.io/arencloud
  tag: "0.1.0"
```

Resolves to `quay.io/arencloud/ambientor-operator:0.1.0`, etc. Local kind/e2e use `registry: ""` → `ambientor-operator:0.1.0`.

## Pull

```bash
helm install ambientor deploy/helm/ambientor/ -n ambientor-system --create-namespace
```

Override registry/tag if needed:

```bash
helm install ambientor deploy/helm/ambientor/ -n ambientor-system --create-namespace \
  --set image.registry=quay.io/arencloud \
  --set image.tag=0.1.0
```

## Local build

```bash
./scripts/lab-build-images.sh
# Quay-style names locally:
# AMBIENTOR_IMAGE_REGISTRY=quay.io/arencloud ./scripts/lab-build-images.sh
```
